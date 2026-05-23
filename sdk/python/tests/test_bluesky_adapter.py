"""Tests for librefang.sidecar.adapters.bluesky.

Deterministic, no network: urllib is monkeypatched. Asserts the
sidecar Bluesky adapter preserves the behaviour of the removed
in-process Rust `librefang-channels::bluesky` adapter, plus two
explicitly-acknowledged improvements:

* P1 (b): outbound threading via in-memory cache (cmd.thread_id →
  reply struct lookup; Rust adapter captured but never used the
  reply ref).
* P2 (b): suppress_error_responses = True (Bluesky posts are public;
  Rust adapter left this as default False).
"""

import io
import json
import os
import time

import pytest

# Required env must be present at import time because the adapter
# raises SystemExit(2) if unset on construction.
os.environ.setdefault("BLUESKY_IDENTIFIER", "test.bsky.social")
os.environ.setdefault("BLUESKY_APP_PASSWORD", "xxxx-xxxx-xxxx-xxxx")
from librefang.sidecar.adapters import bluesky as ba  # noqa: E402

from _sidecar_fakes import _FakeResp, _FakeUrlopen, _HdrShim


def _adapter(**env):
    defaults = {
        "BLUESKY_IDENTIFIER": "test.bsky.social",
        "BLUESKY_APP_PASSWORD": "xxxx-xxxx-xxxx-xxxx",
        "BLUESKY_SERVICE_URL": "",
        "BLUESKY_ACCOUNT_ID": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return ba.BlueskyAdapter()


# ---- env / URL handling -------------------------------------------


def test_default_service_url():
    a = _adapter()
    assert a.service_url == "https://bsky.social"


def test_custom_service_url_strips_trailing_slash():
    a = _adapter(BLUESKY_SERVICE_URL="https://pds.example.com/")
    assert a.service_url == "https://pds.example.com"


def test_missing_required_env_exits():
    with pytest.raises(SystemExit) as exc:
        _adapter(BLUESKY_IDENTIFIER="")
    assert exc.value.code == 2
    with pytest.raises(SystemExit):
        _adapter(BLUESKY_APP_PASSWORD="")


def test_invalid_scheme_rejected():
    with pytest.raises(SystemExit) as exc:
        _adapter(BLUESKY_SERVICE_URL="gemini://bsky.example")
    assert exc.value.code == 2


def test_account_id_optional():
    a = _adapter(BLUESKY_ACCOUNT_ID="prod")
    assert a.account_id == "prod"
    a = _adapter(BLUESKY_ACCOUNT_ID="")
    assert a.account_id is None


# ---- P2 (b): suppress + capabilities ------------------------------


def test_suppress_error_responses_is_true_in_ready_event():
    """P2 (b): explicitly opted into True per maintainer ack. Bluesky
    posts are public; never echo internal errors as a toot."""
    a = _adapter()
    assert a.suppress_error_responses is True
    p = a.ready_event()["params"]
    assert p.get("suppress_error_responses") is True


def test_capabilities_empty():
    a = _adapter()
    assert a.capabilities == []


def test_account_id_in_ready_event():
    a = _adapter(BLUESKY_ACCOUNT_ID="instance-a")
    p = a.ready_event()["params"]
    assert p.get("account_id") == "instance-a"


# ---- _LruCache ---------------------------------------------------


def test_lru_basic_put_get():
    c = ba._LruCache(3)
    c.put("a", {"x": 1})
    assert c.get("a") == {"x": 1}
    assert c.get("missing") is None


def test_lru_evicts_oldest():
    c = ba._LruCache(2)
    c.put("a", {"x": 1})
    c.put("b", {"x": 2})
    c.put("c", {"x": 3})  # evicts "a"
    assert c.get("a") is None
    assert c.get("b") == {"x": 2}
    assert c.get("c") == {"x": 3}


def test_lru_get_marks_recently_used():
    """Touching a key should keep it from being evicted next."""
    c = ba._LruCache(2)
    c.put("a", {"x": 1})
    c.put("b", {"x": 2})
    _ = c.get("a")  # mark a as recent
    c.put("c", {"x": 3})  # should evict b, not a
    assert c.get("a") == {"x": 1}
    assert c.get("b") is None


# ---- _compute_reply_ref -----------------------------------------


def test_compute_reply_ref_direct_mention():
    """For a notification that is itself the start of a thread (no
    record.reply), the reply ref points root and parent at the
    mention itself."""
    notif = {
        "uri": "at://did:plc:alice/app.bsky.feed.post/abc",
        "cid": "bafyabc",
        "record": {"$type": "app.bsky.feed.post", "text": "@bot hi"},
    }
    ref = ba.BlueskyAdapter._compute_reply_ref(notif)
    parent = {"uri": "at://did:plc:alice/app.bsky.feed.post/abc",
              "cid": "bafyabc"}
    assert ref == {"root": parent, "parent": parent}


def test_compute_reply_ref_nested_reply_preserves_root():
    """For a notification that is a reply-to-a-reply, the new reply's
    root must come from the existing record.reply.root (preserving
    the thread origin), while the parent points at the current
    notification."""
    notif = {
        "uri": "at://did:plc:alice/app.bsky.feed.post/reply2",
        "cid": "bafyreply2",
        "record": {
            "$type": "app.bsky.feed.post",
            "text": "@bot another",
            "reply": {
                "root": {"uri": "at://did:plc:alice/app.bsky.feed.post/orig",
                         "cid": "bafyorig"},
                "parent": {"uri": "at://did:plc:alice/app.bsky.feed.post/reply1",
                           "cid": "bafyreply1"},
            },
        },
    }
    ref = ba.BlueskyAdapter._compute_reply_ref(notif)
    assert ref["root"] == {
        "uri": "at://did:plc:alice/app.bsky.feed.post/orig",
        "cid": "bafyorig",
    }
    # Parent is THIS notification, not the prior parent in the chain.
    assert ref["parent"] == {
        "uri": "at://did:plc:alice/app.bsky.feed.post/reply2",
        "cid": "bafyreply2",
    }


# ---- _parse_notification ----------------------------------------


def _notif(reason="mention", text="@bot hello",
           author_did="did:plc:alice", own_did_set=True,
           with_reply=False, uri="at://did:plc:alice/post/1",
           cid="bafy1"):
    return {
        "uri": uri,
        "cid": cid,
        "reason": reason,
        "indexedAt": "2026-05-19T10:00:00.000Z",
        "author": {
            "did": author_did,
            "handle": "alice.bsky.social",
            "displayName": "Alice",
        },
        "record": {
            "$type": "app.bsky.feed.post",
            "text": text,
            **({"reply": {
                "root": {"uri": "at://root/1", "cid": "bafyroot"},
                "parent": {"uri": "at://parent/1", "cid": "bafyparent"},
            }} if with_reply else {}),
        },
    }


def test_parse_notification_mention_full_shape():
    a = _adapter()
    a.own_did = "did:plc:bot"
    notif = _notif()
    ev = a._parse_notification(notif)
    assert ev is not None
    assert ev["method"] == "message"
    p = ev["params"]
    assert p["user_id"] == "did:plc:alice"
    assert p["user_name"] == "Alice"
    assert p["content"] == {"Text": "@bot hello"}
    assert p["message_id"] == "at://did:plc:alice/post/1"
    # thread_id surfaces the URI so daemon round-trips it on outbound.
    assert p["thread_id"] == "at://did:plc:alice/post/1"
    # is_group=False is the default; protocol.message omits the field
    # when False, matching mastodon's behaviour.
    assert "is_group" not in p
    assert p["metadata"]["uri"] == "at://did:plc:alice/post/1"
    assert p["metadata"]["cid"] == "bafy1"
    assert p["metadata"]["handle"] == "alice.bsky.social"
    assert p["metadata"]["reason"] == "mention"
    assert p["metadata"]["indexed_at"] == "2026-05-19T10:00:00.000Z"
    # No record.reply on a fresh mention → no reply_ref in metadata.
    assert "reply_ref" not in p["metadata"]


def test_parse_notification_skips_non_mention_or_reply():
    a = _adapter()
    a.own_did = "did:plc:bot"
    for reason in ("like", "repost", "follow", "quote"):
        assert a._parse_notification(_notif(reason=reason)) is None


def test_parse_notification_accepts_reply_reason():
    a = _adapter()
    a.own_did = "did:plc:bot"
    notif = _notif(reason="reply", with_reply=True)
    ev = a._parse_notification(notif)
    assert ev is not None
    # reply_ref metadata captured for the nested-reply case.
    assert ev["params"]["metadata"]["reply_ref"]["root"]["uri"] == "at://root/1"


def test_parse_notification_skips_self_did():
    a = _adapter()
    a.own_did = "did:plc:bot"
    notif = _notif(author_did="did:plc:bot")
    assert a._parse_notification(notif) is None


def test_parse_notification_skips_empty_text():
    a = _adapter()
    a.own_did = "did:plc:bot"
    notif = _notif(text="")
    assert a._parse_notification(notif) is None


def test_parse_notification_slash_command():
    a = _adapter()
    a.own_did = "did:plc:bot"
    notif = _notif(text="/help me out")
    p = a._parse_notification(notif)["params"]
    assert p["content"] == {
        "Command": {"name": "help", "args": ["me", "out"]}
    }


def test_parse_notification_display_name_falls_back_to_handle():
    a = _adapter()
    a.own_did = "did:plc:bot"
    notif = _notif()
    notif["author"]["displayName"] = ""
    ev = a._parse_notification(notif)
    assert ev["params"]["user_name"] == "alice.bsky.social"


# ---- P1 (b): parse caches reply ref; on_send threads it ----------


def test_parse_caches_reply_ref_for_outbound_threading():
    """P1 (b): parsing a notification stores the computed reply struct
    in the thread cache, keyed by the notification's URI. Outbound
    on_send looks it up via cmd.thread_id and attaches the reply
    field to the createRecord body."""
    a = _adapter()
    a.own_did = "did:plc:bot"
    a._parse_notification(_notif())
    parent = {"uri": "at://did:plc:alice/post/1", "cid": "bafy1"}
    cached = a._thread_cache.get("at://did:plc:alice/post/1")
    assert cached == {"root": parent, "parent": parent}


# ---- _split_message ---------------------------------------------


def test_split_message_under_limit_one_chunk():
    assert ba._split_message("short", 100) == ["short"]


def test_split_message_prefers_newline_cut():
    body = "a" * 80 + "\n" + "b" * 80
    chunks = ba._split_message(body, 100)
    assert len(chunks) == 2
    assert chunks[0] == "a" * 80
    assert chunks[1] == "b" * 80


def test_split_message_hard_cut_when_no_newline():
    chunks = ba._split_message("x" * 250, 100)
    assert [len(c) for c in chunks] == [100, 100, 50]


# ---- session: create / refresh ----------------------------------


def test_create_session_stores_jwt_and_did(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(200, {
        "accessJwt": "access-1",
        "refreshJwt": "refresh-1",
        "did": "did:plc:bot",
    })])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    did = a._create_session()
    assert did == "did:plc:bot"
    assert a._access_jwt == "access-1"
    assert a._refresh_jwt == "refresh-1"
    assert a._session_did == "did:plc:bot"
    # Body sent to createSession is identifier + password.
    assert fake.calls[0]["url"].endswith("/xrpc/com.atproto.server.createSession")
    assert fake.calls[0]["body"] == {
        "identifier": "test.bsky.social",
        "password": "xxxx-xxxx-xxxx-xxxx",
    }


def test_create_session_raises_on_missing_fields(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(200, {"accessJwt": "x"})])  # missing did + refresh
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="missing jwt/did"):
        a._create_session()


def test_refresh_session_falls_back_to_create_on_failure(monkeypatch):
    """The refresh endpoint returning non-200 should trigger a fresh
    createSession with identifier + password — matches Rust behaviour."""
    a = _adapter()
    a._refresh_jwt = "stale-refresh"
    fake = _FakeUrlopen([
        (401, {"error": "ExpiredToken"}),  # refreshSession 401
        (200, {  # fallback createSession succeeds
            "accessJwt": "access-2",
            "refreshJwt": "refresh-2",
            "did": "did:plc:bot",
        }),
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    a._refresh_session()
    assert a._access_jwt == "access-2"
    assert a._refresh_jwt == "refresh-2"
    # Refresh was attempted with stale-refresh; then createSession.
    assert fake.calls[0]["url"].endswith("refreshSession")
    assert fake.calls[0]["headers"]["authorization"] == "Bearer stale-refresh"
    assert fake.calls[1]["url"].endswith("createSession")


# ---- _post_status: createRecord shape ----------------------------


def test_post_status_bearer_auth_and_record_shape(monkeypatch):
    """Outbound creates a record with $type, text, createdAt; bearer
    auth on every request; reply field absent when thread_id is None
    (mirroring the Rust adapter's send() shape)."""
    a = _adapter()
    fake = _FakeUrlopen([
        # createSession during _get_token()
        (200, {
            "accessJwt": "access-1",
            "refreshJwt": "refresh-1",
            "did": "did:plc:bot",
        }),
        # createRecord
        (200, {"uri": "at://did:plc:bot/post/new", "cid": "bafynew"}),
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    a._post_status("hello world", thread_id=None)
    create_call = fake.calls[1]
    assert create_call["url"].endswith("/xrpc/com.atproto.repo.createRecord")
    assert create_call["headers"]["authorization"] == "Bearer access-1"
    body = create_call["body"]
    assert body["repo"] == "did:plc:bot"
    assert body["collection"] == "app.bsky.feed.post"
    rec = body["record"]
    assert rec["$type"] == "app.bsky.feed.post"
    assert rec["text"] == "hello world"
    # createdAt is dynamic; just check shape.
    assert isinstance(rec["createdAt"], str)
    assert rec["createdAt"].endswith("Z")
    # No thread → no reply field.
    assert "reply" not in rec


def test_post_status_p1b_threads_when_thread_id_cached(monkeypatch):
    """P1 (b) integration: when thread_id matches a cached entry, the
    outbound createRecord body MUST include the reply struct."""
    a = _adapter()
    # Pre-populate cache as if a prior parse_notification ran.
    parent = {"uri": "at://did:plc:alice/post/1", "cid": "bafy1"}
    a._thread_cache.put(
        "at://did:plc:alice/post/1",
        {"root": parent, "parent": parent},
    )
    fake = _FakeUrlopen([
        (200, {  # createSession
            "accessJwt": "access-1",
            "refreshJwt": "refresh-1",
            "did": "did:plc:bot",
        }),
        (200, {"uri": "at://did:plc:bot/post/reply", "cid": "bafyreply"}),
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    a._post_status("threaded reply", thread_id="at://did:plc:alice/post/1")
    body = fake.calls[1]["body"]
    assert body["record"]["reply"] == {"root": parent, "parent": parent}


def test_on_send_recovers_uri_from_user_librefang_user(monkeypatch):
    """End-to-end on_send regression guard. The daemon-shape pre-fix
    bug meant cmd.thread_id=None so the cached `{root, parent}` reply
    struct was never looked up, and every reply posted as a top-level
    skeet instead of a thread reply. librefang_user is the always-
    round-tripped carrier — recover from there."""
    import asyncio
    a = _adapter()
    parent = {"uri": "at://did:plc:alice/post/1", "cid": "bafy1"}
    a._thread_cache.put(
        "at://did:plc:alice/post/1",
        {"root": parent, "parent": parent},
    )
    fake = _FakeUrlopen([
        (200, {
            "accessJwt": "access-1",
            "refreshJwt": "refresh-1",
            "did": "did:plc:bot",
        }),
        (200, {"uri": "at://did:plc:bot/post/reply", "cid": "bafyreply"}),
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)

    class _Cmd:
        text = "threaded reply"
        content = {"Text": "threaded reply"}
        thread_id = None  # daemon-default
        user = {
            "platform_id": "alice",
            "librefang_user": "at://did:plc:alice/post/1",
        }

    asyncio.run(a.on_send(_Cmd()))
    body = fake.calls[1]["body"]
    assert body["record"]["reply"] == {"root": parent, "parent": parent}, \
        "on_send must thread via cmd.user.librefang_user when " \
        "cmd.thread_id is None (the daemon default for sidecars " \
        "that don't declare the `thread` capability)"


def test_post_status_cold_cache_recovery_failure_falls_back_to_unthreaded(monkeypatch):
    """Cache miss + XRPC re-fetch also fails (404 / post deleted /
    auth still bad) → fall back to a non-threaded post. Must NOT
    crash; matches the old Rust adapter's degradation."""
    a = _adapter()
    fake = _FakeUrlopen([
        (200, {  # createSession
            "accessJwt": "access-1",
            "refreshJwt": "refresh-1",
            "did": "did:plc:bot",
        }),
        # getPosts re-fetch returns 404 (post deleted between inbound
        # and our outbound retry — no recovery possible).
        (404, {"error": "NotFound"}),
        (200, {"uri": "at://did:plc:bot/post/new", "cid": "bafynew"}),
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    a._post_status("hello", thread_id="at://did:plc:unknown/post/x")
    # Three calls: createSession, getPosts (404), createRecord.
    assert len(fake.calls) == 3
    assert fake.calls[1]["url"].startswith(
        "https://bsky.social/xrpc/app.bsky.feed.getPosts?uris="
    )
    body = fake.calls[2]["body"]
    assert "reply" not in body["record"]


def test_post_status_cache_miss_recovers_reply_ref_via_xrpc(monkeypatch):
    """#5452 fix: when `_thread_cache` was cleared (sidecar restarted
    between the inbound mention and the outbound reply), a single
    XRPC `app.bsky.feed.getPosts?uris=<uri>` re-fetches the post's
    cid and reconstructs the `{root, parent}` reply struct so the
    post still threads instead of becoming a top-level skeet visible
    to all followers' feeds."""
    a = _adapter()
    # Cache is intentionally empty — simulates a fresh sidecar
    # process where the prior _parse_notification's cache write was
    # lost on restart.
    assert len(a._thread_cache) == 0
    fake = _FakeUrlopen([
        (200, {  # createSession
            "accessJwt": "access-1",
            "refreshJwt": "refresh-1",
            "did": "did:plc:bot",
        }),
        # getPosts re-fetch: returns the post (not itself a reply).
        # _recover_reply_ref must derive `{root: parent, parent: parent}`
        # from this single round-trip.
        (200, {
            "posts": [{
                "uri": "at://did:plc:alice/post/1",
                "cid": "bafy1-recovered",
                "record": {
                    "$type": "app.bsky.feed.post",
                    "text": "Hi @bot",
                    "createdAt": "2026-05-21T00:00:00Z",
                },
                "author": {"did": "did:plc:alice", "handle": "alice"},
            }],
        }),
        (200, {"uri": "at://did:plc:bot/post/reply", "cid": "bafyreply"}),
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    a._post_status(
        "threaded reply",
        thread_id="at://did:plc:alice/post/1",
    )
    # getPosts encoded the URI as a query param.
    assert "uris=at%3A%2F%2Fdid%3Aplc%3Aalice%2Fpost%2F1" in fake.calls[1]["url"]
    assert (
        fake.calls[1]["headers"]["authorization"] == "Bearer access-1"
    )
    # createRecord MUST carry the reconstructed reply struct.
    body = fake.calls[2]["body"]
    parent = {"uri": "at://did:plc:alice/post/1", "cid": "bafy1-recovered"}
    assert body["record"]["reply"] == {"root": parent, "parent": parent}
    # Cache must be re-populated so future replies to the same URI
    # don't re-fetch.
    assert a._thread_cache.get("at://did:plc:alice/post/1") == {
        "root": parent, "parent": parent,
    }


def test_post_status_cache_miss_recovery_preserves_existing_thread_root(monkeypatch):
    """When the cache-miss URI points at a post that IS itself a
    reply, recovery MUST use that post's `record.reply.root` (the
    thread's true origin), not the immediate parent — same shape
    as `_compute_reply_ref` produces on the inbound path. Otherwise
    deep-thread replies fork a new sub-thread on every cache miss."""
    a = _adapter()
    root_ref = {
        "uri": "at://did:plc:carol/post/origin",
        "cid": "bafyroot",
    }
    fake = _FakeUrlopen([
        (200, {  # createSession
            "accessJwt": "access-1",
            "refreshJwt": "refresh-1",
            "did": "did:plc:bot",
        }),
        # The fetched post IS a reply in an existing thread.
        (200, {
            "posts": [{
                "uri": "at://did:plc:alice/post/reply",
                "cid": "bafyalice-reply",
                "record": {
                    "$type": "app.bsky.feed.post",
                    "text": "reply text",
                    "createdAt": "2026-05-21T00:00:00Z",
                    "reply": {
                        "root": root_ref,
                        "parent": {
                            "uri": "at://did:plc:bob/post/mid",
                            "cid": "bafybob",
                        },
                    },
                },
            }],
        }),
        (200, {"uri": "at://did:plc:bot/post/reply", "cid": "bafyreply"}),
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    a._post_status(
        "deep reply",
        thread_id="at://did:plc:alice/post/reply",
    )
    body = fake.calls[2]["body"]
    # parent = the post we replied to (cid from the getPosts response).
    expected_parent = {
        "uri": "at://did:plc:alice/post/reply",
        "cid": "bafyalice-reply",
    }
    # root = the thread origin from the fetched post's existing
    # record.reply.root (NOT the post we replied to).
    assert body["record"]["reply"] == {
        "root": root_ref,
        "parent": expected_parent,
    }


def test_post_status_cache_miss_recovery_caches_for_subsequent_chunks(monkeypatch):
    """A multi-chunk reply with cache-miss must trigger EXACTLY ONE
    XRPC re-fetch; subsequent chunks read from the re-populated
    cache. Without this guard a 5-chunk reply would burn 5 extra
    XRPC round-trips per restart-recovered reply."""
    a = _adapter()
    parent = {"uri": "at://did:plc:alice/post/1", "cid": "bafyrec"}
    script = [
        (200, {  # createSession
            "accessJwt": "access-1",
            "refreshJwt": "refresh-1",
            "did": "did:plc:bot",
        }),
        # getPosts re-fetch fires ONCE.
        (200, {
            "posts": [{
                "uri": "at://did:plc:alice/post/1",
                "cid": "bafyrec",
                "record": {
                    "$type": "app.bsky.feed.post",
                    "text": "Hi",
                    "createdAt": "2026-05-21T00:00:00Z",
                },
            }],
        }),
    ]
    # Three createRecord chunks.
    script.extend([
        (200, {"uri": f"at://did:plc:bot/post/{i}", "cid": f"bafy{i}"})
        for i in range(3)
    ])
    fake = _FakeUrlopen(script)
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    a._post_status(
        "x" * (ba.MAX_MESSAGE_LEN * 3),
        thread_id="at://did:plc:alice/post/1",
    )
    # 1 session + 1 getPosts + 3 createRecord = 5 calls. NOT
    # 1 session + 3 × (getPosts + createRecord) = 7 calls.
    assert len(fake.calls) == 5
    # Only call #1 hits getPosts; the rest are createRecord.
    assert "app.bsky.feed.getPosts" in fake.calls[1]["url"]
    for i in range(2, 5):
        assert "createRecord" in fake.calls[i]["url"]
        # And each chunk reuses the reconstructed reply ref.
        assert fake.calls[i]["body"]["record"]["reply"] == {
            "root": parent, "parent": parent,
        }


def test_recover_reply_ref_rejects_non_at_uri():
    """Sanity guard: librefang_user is shared across channels and a
    misrouted value (dingtalk URL, telegram @handle, etc.) must not
    be sent to bsky's XRPC. `_recover_reply_ref` returns None
    without any HTTP call."""
    a = _adapter()
    assert a._recover_reply_ref(
        "https://oapi.dingtalk.com/robot/sendBySession?session=...",
        bearer="access-1",
    ) is None
    assert a._recover_reply_ref("@alice", bearer="access-1") is None
    assert a._recover_reply_ref("", bearer="access-1") is None
    # Even the empty cache must stay empty (no put() side-effect).
    assert len(a._thread_cache) == 0


def test_recover_reply_ref_refreshes_session_and_retries_on_401(monkeypatch):
    """Token rotated between sidecar boot and this outbound — the
    first getPosts gets 401 with the stale bearer; the helper
    invalidates the token (sets `_access_jwt = None`, same pattern
    `_post_status`'s own 401 path uses at line ~487/693) and
    `_get_token` then triggers a fresh `_create_session`. Without
    this, every queued reply after a session rotation would degrade
    to a top-level skeet even though createRecord's own 401 handler
    would later refresh successfully."""
    a = _adapter()
    # Seed a stale access token so _get_token's "still fresh" branch
    # doesn't short-circuit before the refresh.
    a._access_jwt = "stale-token"
    a._session_did = "did:plc:bot"
    a._session_created_at = 0.0
    fake = _FakeUrlopen([
        # getPosts with stale token → 401
        (401, {"error": "ExpiredToken"}),
        # _create_session POST (triggered by _access_jwt = None)
        (200, {
            "accessJwt": "fresh-token",
            "refreshJwt": "refresh-2",
            "did": "did:plc:bot",
        }),
        # getPosts retry with fresh token → 200
        (200, {
            "posts": [{
                "uri": "at://did:plc:alice/post/1",
                "cid": "bafyfresh",
                "record": {
                    "$type": "app.bsky.feed.post",
                    "text": "hi",
                    "createdAt": "2026-05-21T00:00:00Z",
                },
            }],
        }),
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    ref = a._recover_reply_ref(
        "at://did:plc:alice/post/1", bearer="stale-token",
    )
    parent = {"uri": "at://did:plc:alice/post/1", "cid": "bafyfresh"}
    assert ref == {"root": parent, "parent": parent}
    # Three calls: getPosts(401), createSession, getPosts(200).
    assert len(fake.calls) == 3
    assert "getPosts" in fake.calls[0]["url"]
    assert "createSession" in fake.calls[1]["url"]
    assert "getPosts" in fake.calls[2]["url"]
    # The retried getPosts must carry the FRESH token, not the
    # stale one that just got rejected.
    assert fake.calls[2]["headers"]["authorization"] == "Bearer fresh-token"


def test_recover_reply_ref_honours_429_retry_after(monkeypatch):
    """Transient rate limit on getPosts must NOT silently downgrade
    the reply — same pattern the polling + send paths follow
    (`_sleep_on_429_then_raise` family). Honour Retry-After then
    retry once."""
    a = _adapter()
    a._access_jwt = "access-1"
    a._session_did = "did:plc:bot"
    a._session_created_at = time.monotonic()
    fake = _FakeUrlopen([
        # First getPosts → 429 with Retry-After.
        (429, {"error": "RateLimited"}, {"Retry-After": "3"}),
        # Retry getPosts → 200.
        (200, {
            "posts": [{
                "uri": "at://did:plc:alice/post/1",
                "cid": "bafyrl",
                "record": {
                    "$type": "app.bsky.feed.post",
                    "text": "hi",
                    "createdAt": "2026-05-21T00:00:00Z",
                },
            }],
        }),
    ])
    sleeps: list[float] = []
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    monkeypatch.setattr(ba.time, "sleep", sleeps.append)
    ref = a._recover_reply_ref(
        "at://did:plc:alice/post/1", bearer="access-1",
    )
    parent = {"uri": "at://did:plc:alice/post/1", "cid": "bafyrl"}
    assert ref == {"root": parent, "parent": parent}
    assert sleeps == [3.0]
    assert len(fake.calls) == 2


def test_recover_reply_ref_returns_none_on_malformed_posts_response(monkeypatch):
    """Defensive guards: bsky returns 200 but with a posts list
    that contains a non-dict, an empty list, missing cid, or no
    posts key at all — every malformed shape must surface as
    None so the caller degrades cleanly rather than crashing on
    a `.get()` against a non-dict."""
    a = _adapter()
    a._access_jwt = "access-1"
    a._session_did = "did:plc:bot"
    a._session_created_at = time.monotonic()
    for bad_body in (
        {"posts": []},                                   # empty list
        {"posts": [None]},                               # non-dict entry
        {"posts": [{"uri": "at://x/y"}]},                # missing cid
        {"posts": [{"uri": "at://x/y", "cid": ""}]},     # empty cid
        {"otherKey": "value"},                           # no posts key
    ):
        fake = _FakeUrlopen([(200, bad_body)])
        monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
        ref = a._recover_reply_ref(
            "at://did:plc:alice/post/1", bearer="access-1",
        )
        assert ref is None, (
            f"malformed response {bad_body!r} must surface as None; "
            f"got: {ref!r}"
        )


def test_lru_cache_is_thread_safe_under_concurrent_put_get():
    """`_post_status` runs in `run_in_executor(None, ...)` which
    uses the default ThreadPoolExecutor — concurrent `put` / `get`
    from worker threads is real. Without a lock the `move_to_end +
    assignment + popitem` sequence races on OrderedDict's internal
    linked list, dropping/duplicating entries. This test hammers
    the cache from 8 threads doing 200 puts each to catch the
    worst-case interleaving (deterministic enough to spot
    corruption — pure data-race tests aren't always — but
    consistent failure if the lock is removed)."""
    import threading as _threading
    cache = ba._LruCache(max_size=50)
    threads = []
    errors: list[str] = []

    def writer(prefix: str) -> None:
        try:
            for i in range(200):
                cache.put(f"{prefix}-{i}", {"i": i})
                # Interleave reads with writes to stress
                # move_to_end vs popitem ordering.
                cache.get(f"{prefix}-{max(0, i - 5)}")
        except Exception as e:
            errors.append(f"{prefix}: {e!r}")

    for k in range(8):
        t = _threading.Thread(target=writer, args=(f"w{k}",), daemon=True)
        threads.append(t)
        t.start()
    for t in threads:
        t.join(timeout=10.0)
        assert not t.is_alive(), "writer hung — possible lock deadlock"
    assert errors == [], (
        f"thread-safety regression: cache mutator raised under "
        f"concurrent access — {errors}"
    )
    # Cache size must respect the cap regardless of concurrent
    # writer interleavings. Without the lock, `popitem` could be
    # skipped or the dict length could blow past max_size.
    assert len(cache) <= 50, (
        f"cache cap violated under concurrent writes: len={len(cache)}"
    )


def test_post_status_chunks_keep_same_reply_ref(monkeypatch):
    """When the message exceeds MAX_MESSAGE_LEN, every chunk reuses
    the same reply struct so the multi-part reply stays under one
    thread parent — improvement over the Rust adapter which never
    threaded any chunk."""
    a = _adapter()
    parent = {"uri": "at://did:plc:alice/post/1", "cid": "bafy1"}
    a._thread_cache.put(
        "at://did:plc:alice/post/1",
        {"root": parent, "parent": parent},
    )
    script = [
        (200, {
            "accessJwt": "access-1",
            "refreshJwt": "refresh-1",
            "did": "did:plc:bot",
        }),
    ]
    # Two createRecord calls (text is 2x the cap)
    script.extend([
        (200, {"uri": f"at://did:plc:bot/post/{i}", "cid": f"bafy{i}"})
        for i in range(2)
    ])
    fake = _FakeUrlopen(script)
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    a._post_status(
        "x" * (ba.MAX_MESSAGE_LEN + 50),
        thread_id="at://did:plc:alice/post/1",
    )
    # Calls[0] = createSession; calls[1] and [2] = createRecord chunks.
    assert len(fake.calls) == 3
    for create_call in fake.calls[1:]:
        assert create_call["body"]["record"]["reply"] == {
            "root": parent, "parent": parent,
        }


def test_post_status_5xx_surfaced(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([
        (200, {
            "accessJwt": "access-1",
            "refreshJwt": "refresh-1",
            "did": "did:plc:bot",
        }),
        (500, {"error": "InternalServerError"}),
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="500"):
        a._post_status("hi", thread_id=None)


def test_post_status_401_retries_with_fresh_session(monkeypatch):
    """createRecord 401 should clear the session and retry once with a
    fresh access token. Mirrors the Rust adapter's auth-recovery loop
    inside the polling path; we apply it on outbound too because the
    Rust adapter would also re-create on 401 via get_token() under the
    same conditions (session.created_at-based refresh + drop)."""
    a = _adapter()
    fake = _FakeUrlopen([
        (200, {  # initial createSession
            "accessJwt": "access-1",
            "refreshJwt": "refresh-1",
            "did": "did:plc:bot",
        }),
        (401, {"error": "ExpiredToken"}),  # first createRecord fails
        (200, {  # retry createSession (refresh path will fall back)
            "accessJwt": "access-2",
            "refreshJwt": "refresh-2",
            "did": "did:plc:bot",
        }),
        (200, {"uri": "at://did:plc:bot/post/retry", "cid": "bafyretry"}),
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    a._post_status("retry me", thread_id=None)
    # Final successful createRecord uses the refreshed access-2 token.
    final = fake.calls[-1]
    assert final["url"].endswith("createRecord")
    assert final["headers"]["authorization"] == "Bearer access-2"


# ---- _poll_once: paging + 401 clears session --------------------


def test_poll_once_emits_parsed_notifications(monkeypatch):
    a = _adapter()
    a.own_did = "did:plc:bot"
    # Pre-warm session to avoid the implicit createSession in _get_token.
    a._access_jwt = "access-1"
    a._refresh_jwt = "refresh-1"
    a._session_did = "did:plc:bot"
    a._session_created_at = ba.time.monotonic()

    fake = _FakeUrlopen([
        (200, {
            "notifications": [
                _notif(text="@bot hello", uri="at://post/A", cid="bafyA"),
                _notif(reason="like", text=""),  # skipped
            ],
        }),
        (200, {}),  # updateSeen
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)

    emitted: list[dict] = []
    new_seen = a._poll_once(emitted.append, last_seen_at=None)
    assert len(emitted) == 1
    assert emitted[0]["params"]["message_id"] == "at://post/A"
    assert new_seen == "2026-05-19T10:00:00.000Z"
    # First call: listNotifications; second: updateSeen.
    assert "listNotifications" in fake.calls[0]["url"]
    assert "updateSeen" in fake.calls[1]["url"]


def test_poll_once_401_clears_session_and_raises(monkeypatch):
    a = _adapter()
    a.own_did = "did:plc:bot"
    a._access_jwt = "stale-access"
    a._refresh_jwt = "stale-refresh"
    a._session_did = "did:plc:bot"
    a._session_created_at = ba.time.monotonic()
    fake = _FakeUrlopen([(401, {"error": "ExpiredToken"})])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="401"):
        a._poll_once(lambda _: None, last_seen_at=None)
    # Mirrors Rust: 401 clears the session so the next poll re-auths.
    assert a._access_jwt is None


def test_poll_once_seenAt_query_param_when_set(monkeypatch):
    a = _adapter()
    a.own_did = "did:plc:bot"
    a._access_jwt = "access-1"
    a._refresh_jwt = "refresh-1"
    a._session_did = "did:plc:bot"
    a._session_created_at = ba.time.monotonic()
    fake = _FakeUrlopen([(200, {"notifications": []})])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    a._poll_once(lambda _: None, last_seen_at="2026-05-19T09:00:00.000Z")
    url = fake.calls[0]["url"]
    assert "seenAt=2026-05-19T09" in url
    assert "limit=25" in url


def test_poll_once_emits_in_chronological_order(monkeypatch):
    """Regression: `listNotifications` returns notifications
    newest-first. A burst caught in one poll must reach the agent
    oldest -> newest, not reversed (the Rust adapter iterated the raw
    newest-first list). The high-water mark is the max `indexedAt` and
    so is independent of emit order."""
    a = _adapter()
    a.own_did = "did:plc:bot"
    a._access_jwt = "access-1"
    a._refresh_jwt = "refresh-1"
    a._session_did = "did:plc:bot"
    a._session_created_at = ba.time.monotonic()

    def _n(uri, text, indexed):
        n = _notif(text=text, uri=uri, cid="bafy" + uri[-1])
        n["indexedAt"] = indexed
        return n

    fake = _FakeUrlopen([
        (200, {"notifications": [  # API order: newest-first
            _n("at://post/C", "@bot third", "2026-05-19T10:00:30.000Z"),
            _n("at://post/B", "@bot second", "2026-05-19T10:00:20.000Z"),
            _n("at://post/A", "@bot first", "2026-05-19T10:00:10.000Z"),
        ]}),
        (200, {}),  # updateSeen
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    emitted: list[dict] = []
    new_seen = a._poll_once(emitted.append, last_seen_at=None)
    assert [e["params"]["message_id"] for e in emitted] == [
        "at://post/A", "at://post/B", "at://post/C",
    ]
    # High-water mark = max indexedAt, order-independent.
    assert new_seen == "2026-05-19T10:00:30.000Z"


# ---- 429 / Retry-After (XRPC rate-limiting) ---------------------


def test_retry_after_secs_parses_header_value():
    """``Retry-After`` (seconds form) is parsed as a float and capped
    at ``MAX_BACKOFF_SECS`` so a misreported value can't block the
    poller for more than a minute."""
    assert ba.BlueskyAdapter._retry_after_secs({"retry-after": "5"}) == 5.0
    assert ba.BlueskyAdapter._retry_after_secs({"retry-after": "0.5"}) == 1.0
    assert (
        ba.BlueskyAdapter._retry_after_secs({"retry-after": "9999"})
        == ba.MAX_BACKOFF_SECS
    )


def test_retry_after_secs_falls_back_when_absent_or_invalid():
    """Without a ``Retry-After`` (or with an unparseable HTTP-date /
    ``RateLimit-Reset`` epoch we don't decode), fall back to
    ``RETRY_AFTER_DEFAULT_SECS`` rather than busy-looping at 1 s."""
    assert (
        ba.BlueskyAdapter._retry_after_secs({})
        == ba.RETRY_AFTER_DEFAULT_SECS
    )
    assert (
        ba.BlueskyAdapter._retry_after_secs(
            {"retry-after": "Thu, 01 Jan 2099 00:00:00 GMT"},
        )
        == ba.RETRY_AFTER_DEFAULT_SECS
    )


def test_create_session_429_sleeps_retry_after_then_raises(monkeypatch):
    """PDS rate-limits failed createSession aggressively from a single
    IP. The sidecar must honour ``Retry-After`` here — otherwise the
    producer's verify-credentials retry loop compounds with the server-
    side block."""
    a = _adapter()
    fake = _FakeUrlopen([
        (429, {"error": "RateLimitExceeded"}, {"Retry-After": "3"}),
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(ba.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._create_session()
    assert sleeps == [3.0]


def test_create_session_429_without_header_uses_default(monkeypatch):
    """A 429 with no ``Retry-After`` falls back to
    ``RETRY_AFTER_DEFAULT_SECS`` instead of busy-looping at 1 s."""
    a = _adapter()
    fake = _FakeUrlopen([(429, {"error": "RateLimitExceeded"})])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(ba.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._create_session()
    assert sleeps == [ba.RETRY_AFTER_DEFAULT_SECS]


def test_refresh_session_429_sleeps_retry_after_then_raises(monkeypatch):
    """``refreshSession`` is throttled on the same envelope as
    ``createSession``; the sidecar honours ``Retry-After`` here too so
    a token-near-expiry probe doesn't extend the throttling window."""
    a = _adapter()
    a._access_jwt = "old"
    a._refresh_jwt = "refresh-token"
    a._session_did = "did:plc:bot"
    a._session_created_at = 0.0
    fake = _FakeUrlopen([
        (429, {"error": "RateLimitExceeded"}, {"Retry-After": "4"}),
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(ba.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._refresh_session()
    assert sleeps == [4.0]


def test_poll_once_429_sleeps_retry_after_then_raises(monkeypatch):
    """``listNotifications`` 429 must sleep the indicated interval and
    raise so the outer backoff in `_producer_blocking` pauses before
    the next pass — otherwise the poll loop probes inside the window
    and extends the throttling."""
    a = _adapter()
    a.own_did = "did:plc:bot"
    a._access_jwt = "access-1"
    a._refresh_jwt = "refresh-1"
    a._session_did = "did:plc:bot"
    a._session_created_at = ba.time.monotonic()
    fake = _FakeUrlopen([
        (429, {"error": "RateLimitExceeded"}, {"Retry-After": "7"}),
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(ba.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._poll_once(lambda _: None, last_seen_at=None)
    assert sleeps == [7.0]


def test_post_status_429_sleeps_retry_after_then_raises(monkeypatch):
    """``createRecord`` is rate-limited independently of auth. A 429
    here must sleep and raise; `suppress_error_responses=True` keeps
    the raise from echoing back as a public post."""
    a = _adapter()
    a._access_jwt = "access-1"
    a._refresh_jwt = "refresh-1"
    a._session_did = "did:plc:bot"
    a._session_created_at = ba.time.monotonic()
    fake = _FakeUrlopen([
        (429, {"error": "RateLimitExceeded"}, {"Retry-After": "6"}),
    ])
    monkeypatch.setattr(ba.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(ba.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._post_status("hello", thread_id=None)
    assert sleeps == [6.0]
