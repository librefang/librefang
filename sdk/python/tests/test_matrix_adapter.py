"""Tests for librefang.sidecar.adapters.matrix.

Deterministic, no network: urllib is monkeypatched against the
shared _FakeUrlopen helper. Asserts the sidecar preserves the
in-process Rust ``librefang-channels::matrix`` adapter's behaviour
plus the three improvements documented in the module header
(inbound dedupe, 429 honoured everywhere, explicit timeouts).
"""

import json
import os

import pytest

from _sidecar_fakes import _FakeResp, _FakeUrlopen, _HdrShim


os.environ.setdefault("MATRIX_HOMESERVER_URL", "https://matrix.test")
os.environ.setdefault("MATRIX_USER_ID", "@bot:matrix.test")
os.environ.setdefault("MATRIX_ACCESS_TOKEN", "syt_test_token")
from librefang.sidecar.adapters import matrix as mx  # noqa: E402


def _adapter(**env):
    defaults = {
        "MATRIX_HOMESERVER_URL": "https://matrix.test",
        "MATRIX_USER_ID": "@bot:matrix.test",
        "MATRIX_ACCESS_TOKEN": "syt_test_token",
        "MATRIX_ALLOWED_ROOMS": "",
        "MATRIX_ACCOUNT_ID": "",
        "MATRIX_MAX_UPLOAD_BYTES": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return mx.MatrixAdapter()


# ---- env handling ----------------------------------------------------


def test_default_env_construction():
    a = _adapter()
    assert a.homeserver_url == "https://matrix.test"
    assert a.user_id == "@bot:matrix.test"
    assert a.access_token == "syt_test_token"
    assert a.allowed_rooms == []
    assert a.account_id is None
    assert a.max_upload_bytes == mx.DEFAULT_MAX_UPLOAD_BYTES


def test_homeserver_trailing_slash_stripped():
    a = _adapter(MATRIX_HOMESERVER_URL="https://matrix.test/")
    assert a.homeserver_url == "https://matrix.test"


def test_allowed_rooms_csv_split():
    a = _adapter(MATRIX_ALLOWED_ROOMS="!abc:m.org, !def:m.org ,, !ghi:m.org")
    assert a.allowed_rooms == ["!abc:m.org", "!def:m.org", "!ghi:m.org"]


def test_account_id_passthrough():
    a = _adapter(MATRIX_ACCOUNT_ID="prod-bot")
    assert a.account_id == "prod-bot"


def test_account_id_empty_is_none():
    a = _adapter(MATRIX_ACCOUNT_ID="")
    assert a.account_id is None


def test_max_upload_bytes_override():
    a = _adapter(MATRIX_MAX_UPLOAD_BYTES="1048576")
    assert a.max_upload_bytes == 1024 * 1024


def test_max_upload_bytes_garbage_falls_back():
    a = _adapter(MATRIX_MAX_UPLOAD_BYTES="not-a-number")
    assert a.max_upload_bytes == mx.DEFAULT_MAX_UPLOAD_BYTES


def test_missing_homeserver_exits_2():
    os.environ["MATRIX_HOMESERVER_URL"] = ""
    with pytest.raises(SystemExit) as exc:
        mx.MatrixAdapter()
    assert exc.value.code == 2
    os.environ["MATRIX_HOMESERVER_URL"] = "https://matrix.test"


def test_missing_user_id_exits_2():
    os.environ["MATRIX_USER_ID"] = ""
    with pytest.raises(SystemExit) as exc:
        mx.MatrixAdapter()
    assert exc.value.code == 2
    os.environ["MATRIX_USER_ID"] = "@bot:matrix.test"


def test_missing_access_token_exits_2():
    os.environ["MATRIX_ACCESS_TOKEN"] = ""
    with pytest.raises(SystemExit) as exc:
        mx.MatrixAdapter()
    assert exc.value.code == 2
    os.environ["MATRIX_ACCESS_TOKEN"] = "syt_test_token"


def test_non_http_scheme_exits_2():
    os.environ["MATRIX_HOMESERVER_URL"] = "ws://matrix.test"
    with pytest.raises(SystemExit) as exc:
        mx.MatrixAdapter()
    assert exc.value.code == 2
    os.environ["MATRIX_HOMESERVER_URL"] = "https://matrix.test"


# ---- mxc_to_http -----------------------------------------------------


def test_mxc_to_http_basic():
    out = mx.mxc_to_http("mxc://m.org/abc123", "https://matrix.test")
    assert out == "https://matrix.test/_matrix/client/v1/media/download/m.org/abc123"


def test_mxc_to_http_trailing_slash_homeserver():
    out = mx.mxc_to_http("mxc://m.org/abc", "https://matrix.test/")
    assert out == "https://matrix.test/_matrix/client/v1/media/download/m.org/abc"


def test_mxc_to_http_rejects_non_mxc():
    assert mx.mxc_to_http("http://m.org/x", "https://matrix.test") is None
    assert mx.mxc_to_http("mxc://m.org", "https://matrix.test") is None
    assert mx.mxc_to_http("mxc:///media", "https://matrix.test") is None
    assert mx.mxc_to_http("mxc://m.org/", "https://matrix.test") is None


# ---- markdown_to_matrix_html ----------------------------------------


def test_markdown_inline_bold_italic_code():
    h = mx.markdown_to_matrix_html("**bold** *italic* `code`")
    assert "<strong>bold</strong>" in h
    assert "<em>italic</em>" in h
    assert "<code>code</code>" in h


def test_markdown_headings():
    h = mx.markdown_to_matrix_html("# h1\n## h2\n### h3")
    assert "<h1>h1</h1>" in h
    assert "<h2>h2</h2>" in h
    assert "<h3>h3</h3>" in h


def test_markdown_links():
    h = mx.markdown_to_matrix_html("[label](https://example.com)")
    assert '<a href="https://example.com">label</a>' in h


def test_markdown_rejects_javascript_link():
    """javascript: / data: URLs in the source MUST NOT survive into
    the rendered <a href=""> — that's an XSS escape hatch otherwise."""
    h = mx.markdown_to_matrix_html("[x](javascript:alert(1))")
    assert "<a href=" not in h
    h2 = mx.markdown_to_matrix_html("[x](data:text/html,<x>)")
    assert "<a href=" not in h2


def test_markdown_lists_ul():
    h = mx.markdown_to_matrix_html("- a\n- b\n- c")
    assert "<ul>" in h
    assert "<li>a</li>" in h
    assert "<li>b</li>" in h
    assert "</ul>" in h


def test_markdown_lists_ol():
    h = mx.markdown_to_matrix_html("1. one\n2. two")
    assert "<ol>" in h
    assert "<li>one</li>" in h
    assert "<li>two</li>" in h


def test_markdown_blockquote():
    h = mx.markdown_to_matrix_html("> quoted line")
    assert "<blockquote>" in h
    assert "quoted line" in h


def test_markdown_code_block_fenced():
    h = mx.markdown_to_matrix_html("```\nfoo bar\n```")
    assert "<pre><code>" in h
    assert "foo bar" in h


def test_markdown_code_block_with_language():
    h = mx.markdown_to_matrix_html("```python\nprint(1)\n```")
    assert 'class="language-python"' in h
    assert "print(1)" in h


def test_markdown_horizontal_rule():
    h = mx.markdown_to_matrix_html("before\n\n---\n\nafter")
    assert "<hr/>" in h


def test_markdown_table():
    h = mx.markdown_to_matrix_html("| a | b |\n|---|---|\n| 1 | 2 |")
    assert "<table>" in h
    assert "<th>a</th>" in h
    assert "<td>1</td>" in h


def test_markdown_strikethrough():
    h = mx.markdown_to_matrix_html("~~struck~~")
    assert "<del>struck</del>" in h


def test_markdown_html_escape_in_source():
    """A model emitting raw <script> in its response must NOT inject
    markup into formatted_body. The rendered HTML must contain
    &lt;script&gt; not <script>."""
    h = mx.markdown_to_matrix_html("plain <script>alert(1)</script>")
    assert "<script>" not in h
    assert "&lt;script&gt;" in h


def test_markdown_strips_think_block():
    """<think>...</think> LLM reasoning artefacts are stripped first."""
    h = mx.markdown_to_matrix_html("<think>internal</think>actual reply")
    assert "internal" not in h
    assert "actual reply" in h


def test_markdown_empty_input():
    assert mx.markdown_to_matrix_html("") == ""


# ---- text_body_with_html --------------------------------------------


def test_text_body_with_html_basic():
    v = mx.text_body_with_html("**bold**")
    assert v["msgtype"] == "m.text"
    assert v["body"] == "**bold**"
    assert v["format"] == "org.matrix.custom.html"
    assert "<strong>bold</strong>" in v["formatted_body"]


def test_text_body_with_html_merges_extras():
    extras = {"m.relates_to": {"rel_type": "m.thread", "event_id": "$x"}}
    v = mx.text_body_with_html("hi", extras)
    assert v["m.relates_to"]["rel_type"] == "m.thread"
    assert v["m.relates_to"]["event_id"] == "$x"


# ---- build_edit_body -------------------------------------------------


def test_build_edit_body_shape():
    v = mx.build_edit_body("$target", "new text")
    assert v["msgtype"] == "m.text"
    assert v["body"] == "* new text"
    assert v["m.new_content"]["body"] == "new text"
    assert v["m.relates_to"]["rel_type"] == "m.replace"
    assert v["m.relates_to"]["event_id"] == "$target"


def test_build_edit_body_truncates_long_text():
    """``body`` / ``m.new_content.body`` is capped at MAX_MESSAGE_LEN
    (formatted_body is allowed to overflow because truncating HTML
    can leave half-open tags)."""
    long_text = "x" * (mx.MAX_MESSAGE_LEN + 100)
    v = mx.build_edit_body("$t", long_text)
    assert len(v["m.new_content"]["body"]) == mx.MAX_MESSAGE_LEN


# ---- parse_thread_relation ------------------------------------------


def test_parse_thread_relation_present():
    content = {
        "m.relates_to": {
            "rel_type": "m.thread",
            "event_id": "$root",
        },
    }
    assert mx.parse_thread_relation(content) == "$root"


def test_parse_thread_relation_absent_for_plain():
    assert mx.parse_thread_relation({"body": "hi"}) is None


def test_parse_thread_relation_absent_for_replace():
    """An edit's ``m.replace`` is not a thread — return None."""
    content = {
        "m.relates_to": {"rel_type": "m.replace", "event_id": "$x"},
    }
    assert mx.parse_thread_relation(content) is None


def test_parse_thread_relation_handles_malformed():
    assert mx.parse_thread_relation(None) is None
    assert mx.parse_thread_relation({"m.relates_to": "string"}) is None
    assert mx.parse_thread_relation({"m.relates_to": {}}) is None


# ---- parse_inbound_msg_content --------------------------------------


def test_parse_inbound_text_message():
    content = {"msgtype": "m.text", "body": "hello world"}
    out = mx.parse_inbound_msg_content(content, "https://matrix.test")
    assert out == {"Text": "hello world"}


def test_parse_inbound_text_slash_command():
    content = {"msgtype": "m.text", "body": "/status all systems"}
    out = mx.parse_inbound_msg_content(content, "https://matrix.test")
    assert out == {"Command": {"name": "status", "args": ["all", "systems"]}}


def test_parse_inbound_notice_treated_as_text():
    content = {"msgtype": "m.notice", "body": "from a bot"}
    out = mx.parse_inbound_msg_content(content, "https://matrix.test")
    assert out == {"Text": "from a bot"}


def test_parse_inbound_emote_treated_as_text():
    content = {"msgtype": "m.emote", "body": "waves"}
    out = mx.parse_inbound_msg_content(content, "https://matrix.test")
    assert out == {"Text": "waves"}


def test_parse_inbound_default_msgtype_is_text():
    """Missing msgtype defaults to m.text per matrix.rs:318."""
    content = {"body": "implicit text"}
    out = mx.parse_inbound_msg_content(content, "https://matrix.test")
    assert out == {"Text": "implicit text"}


def test_parse_inbound_empty_body_returns_none():
    content = {"msgtype": "m.text", "body": ""}
    assert mx.parse_inbound_msg_content(content, "https://matrix.test") is None


def test_parse_inbound_image_event():
    content = {
        "msgtype": "m.image",
        "body": "cat.jpg",
        "url": "mxc://m.test/abc",
        "info": {"mimetype": "image/jpeg"},
    }
    out = mx.parse_inbound_msg_content(content, "https://matrix.test")
    assert out is not None
    assert "Image" in out
    assert out["Image"]["mime_type"] == "image/jpeg"


def test_parse_inbound_file_filename_wins_over_body():
    """Matrix v1.10+ ``filename`` takes precedence over ``body``."""
    content = {
        "msgtype": "m.file",
        "body": "fallback.txt",
        "filename": "actual.pdf",
        "url": "mxc://m.test/file",
    }
    out = mx.parse_inbound_msg_content(content, "https://matrix.test")
    assert out["File"]["filename"] == "actual.pdf"


def test_parse_inbound_audio_voice_msc3245():
    """``org.matrix.msc3245.voice`` promotes m.audio to Voice."""
    content = {
        "msgtype": "m.audio",
        "body": "voice note",
        "url": "mxc://m.test/v",
        "info": {"duration": 5000},
        "org.matrix.msc3245.voice": {},
    }
    out = mx.parse_inbound_msg_content(content, "https://matrix.test")
    assert "Voice" in out
    assert out["Voice"]["duration_seconds"] == 5


def test_parse_inbound_audio_plain():
    content = {
        "msgtype": "m.audio",
        "body": "song",
        "url": "mxc://m.test/a",
        "info": {"duration": 30000},
    }
    out = mx.parse_inbound_msg_content(content, "https://matrix.test")
    assert "Audio" in out
    assert out["Audio"]["duration_seconds"] == 30


def test_parse_inbound_video():
    content = {
        "msgtype": "m.video",
        "body": "clip.mp4",
        "url": "mxc://m.test/v",
        "info": {"duration": 60_000},
    }
    out = mx.parse_inbound_msg_content(content, "https://matrix.test")
    assert "Video" in out
    assert out["Video"]["duration_seconds"] == 60


def test_parse_inbound_unknown_msgtype_returns_none():
    content = {"msgtype": "m.location", "body": "where"}
    assert mx.parse_inbound_msg_content(content, "https://matrix.test") is None


def test_parse_inbound_missing_url_returns_none():
    content = {"msgtype": "m.image", "body": "no url"}
    assert mx.parse_inbound_msg_content(content, "https://matrix.test") is None


# ---- /sync body processing ------------------------------------------


def _sync_body(events, room_id="!room:m.test", next_batch="b1"):
    return {
        "next_batch": next_batch,
        "rooms": {
            "join": {
                room_id: {
                    "timeline": {"events": events, "limit": 10},
                },
            },
        },
    }


def test_process_sync_emits_text_message():
    a = _adapter()
    body = _sync_body([{
        "type": "m.room.message",
        "event_id": "$e1",
        "sender": "@alice:m.test",
        "content": {"msgtype": "m.text", "body": "hi"},
    }])
    emitted = []
    a._process_sync_body(body, emitted.append)
    assert len(emitted) == 1
    p = emitted[0]["params"]
    assert p["user_id"] == "!room:m.test"
    assert p["user_name"] == "@alice:m.test"
    assert p["channel_id"] == "!room:m.test"
    assert p["message_id"] == "$e1"
    assert p["content"] == {"Text": "hi"}
    assert p["is_group"] is True
    assert a.since_token == "b1"


def test_process_sync_self_skip():
    a = _adapter()
    body = _sync_body([{
        "type": "m.room.message",
        "event_id": "$e1",
        "sender": "@bot:matrix.test",  # bot's own user_id
        "content": {"msgtype": "m.text", "body": "echo"},
    }])
    emitted = []
    a._process_sync_body(body, emitted.append)
    assert emitted == []


def test_process_sync_room_allowlist_skip():
    a = _adapter(MATRIX_ALLOWED_ROOMS="!allowed:m.test")
    body = _sync_body([{
        "type": "m.room.message",
        "event_id": "$e1",
        "sender": "@alice:m.test",
        "content": {"msgtype": "m.text", "body": "hi"},
    }], room_id="!other:m.test")
    emitted = []
    a._process_sync_body(body, emitted.append)
    assert emitted == []


def test_process_sync_room_allowlist_pass():
    a = _adapter(MATRIX_ALLOWED_ROOMS="!allowed:m.test")
    body = _sync_body([{
        "type": "m.room.message",
        "event_id": "$e2",
        "sender": "@alice:m.test",
        "content": {"msgtype": "m.text", "body": "hi"},
    }], room_id="!allowed:m.test")
    emitted = []
    a._process_sync_body(body, emitted.append)
    assert len(emitted) == 1


def test_process_sync_dedupes_repeated_event_id():
    a = _adapter()
    ev = {
        "type": "m.room.message",
        "event_id": "$dup",
        "sender": "@alice:m.test",
        "content": {"msgtype": "m.text", "body": "hi"},
    }
    body1 = _sync_body([ev], next_batch="b1")
    body2 = _sync_body([ev], next_batch="b2")
    emitted = []
    a._process_sync_body(body1, emitted.append)
    a._process_sync_body(body2, emitted.append)
    assert len(emitted) == 1  # second occurrence deduped


def test_process_sync_e2ee_warn_once_no_emit():
    a = _adapter()
    body = _sync_body([{
        "type": "m.room.encrypted",
        "event_id": "$enc",
        "sender": "@alice:m.test",
        "content": {"algorithm": "m.megolm.v1.aes-sha2"},
    }])
    emitted = []
    a._process_sync_body(body, emitted.append)
    assert emitted == []
    # Second pass on same room — internal warn-once tracking pin.
    assert "!room:m.test" in a._e2ee_warned


def test_process_sync_skips_non_room_message_event():
    a = _adapter()
    body = _sync_body([{
        "type": "m.room.member",
        "event_id": "$m",
        "sender": "@alice:m.test",
        "content": {"membership": "join"},
    }])
    emitted = []
    a._process_sync_body(body, emitted.append)
    assert emitted == []


def test_process_sync_thread_relation_surfaces_thread_id():
    a = _adapter()
    body = _sync_body([{
        "type": "m.room.message",
        "event_id": "$e_thread",
        "sender": "@alice:m.test",
        "content": {
            "msgtype": "m.text",
            "body": "reply",
            "m.relates_to": {
                "rel_type": "m.thread",
                "event_id": "$root",
            },
        },
    }])
    emitted = []
    a._process_sync_body(body, emitted.append)
    assert emitted[0]["params"]["thread_id"] == "$root"


def test_process_sync_account_id_injected():
    a = _adapter(MATRIX_ACCOUNT_ID="prod")
    body = _sync_body([{
        "type": "m.room.message",
        "event_id": "$e",
        "sender": "@alice:m.test",
        "content": {"msgtype": "m.text", "body": "hi"},
    }])
    emitted = []
    a._process_sync_body(body, emitted.append)
    assert emitted[0]["params"]["metadata"]["account_id"] == "prod"


# ---- reaction lifecycle cache ---------------------------------------


def test_phase_reaction_insert_and_lookup():
    a = _adapter()
    key = ("!r", "$target")
    a._phase_reaction_insert(key, "$reaction-1")
    assert a._phase_reaction_lookup(key) == "$reaction-1"


def test_phase_reaction_replace_in_place():
    a = _adapter()
    key = ("!r", "$target")
    a._phase_reaction_insert(key, "$reaction-1")
    a._phase_reaction_insert(key, "$reaction-2")
    assert a._phase_reaction_lookup(key) == "$reaction-2"


def test_phase_reaction_remove():
    a = _adapter()
    key = ("!r", "$target")
    a._phase_reaction_insert(key, "$reaction-1")
    assert a._phase_reaction_remove(key) == "$reaction-1"
    assert a._phase_reaction_lookup(key) is None


def test_phase_reaction_capacity_eviction(monkeypatch):
    monkeypatch.setattr(mx, "PHASE_REACTIONS_CAPACITY", 3)
    a = _adapter()
    for i in range(4):
        a._phase_reaction_insert(("!r", f"$t{i}"), f"$react{i}")
    # Oldest ($t0) was evicted; the rest remain.
    assert a._phase_reaction_lookup(("!r", "$t0")) is None
    assert a._phase_reaction_lookup(("!r", "$t3")) == "$react3"


# ---- _put_event 429 retry -------------------------------------------


def test_put_event_happy_path(monkeypatch):
    fake = _FakeUrlopen([(200, {"event_id": "$srv-id"})])
    monkeypatch.setattr(mx.urllib.request, "urlopen", fake)
    a = _adapter()
    eid = a._put_event("!r:m.test", "m.room.message", {"msgtype": "m.text"})
    assert eid == "$srv-id"
    c = fake.calls[0]
    assert c["method"] == "PUT"
    assert "/_matrix/client/v3/rooms/" in c["url"]
    assert "/send/m.room.message/" in c["url"]
    assert c["headers"]["authorization"] == "Bearer syt_test_token"


def test_put_event_429_then_200(monkeypatch):
    sleeps = []
    monkeypatch.setattr(mx.time, "sleep", lambda s: sleeps.append(s))
    fake = _FakeUrlopen([
        (429, {}, {"Retry-After": "1"}),
        (200, {"event_id": "$srv"}),
    ])
    monkeypatch.setattr(mx.urllib.request, "urlopen", fake)
    a = _adapter()
    eid = a._put_event("!r", "m.room.message", {"msgtype": "m.text"})
    assert eid == "$srv"
    assert sleeps == [1.0]
    assert len(fake.calls) == 2


def test_put_event_persistent_429_raises(monkeypatch):
    monkeypatch.setattr(mx.time, "sleep", lambda _s: None)
    fake = _FakeUrlopen([
        (429, {}, {}),
        (429, {}, {}),
    ])
    monkeypatch.setattr(mx.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="rate-limited persistently"):
        a._put_event("!r", "m.room.message", {})


def test_put_event_non_2xx_raises(monkeypatch):
    fake = _FakeUrlopen([(404, {"errcode": "M_NOT_FOUND"})])
    monkeypatch.setattr(mx.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="status=404"):
        a._put_event("!r", "m.room.message", {})


# ---- _upload_media --------------------------------------------------


def test_upload_media_returns_mxc(monkeypatch):
    fake = _FakeUrlopen([(200, {"content_uri": "mxc://m.test/uploaded"})])
    monkeypatch.setattr(mx.urllib.request, "urlopen", fake)
    a = _adapter()
    mxc = a._upload_media(b"hello", "x.txt", "text/plain")
    assert mxc == "mxc://m.test/uploaded"
    c = fake.calls[0]
    assert c["method"] == "POST"
    assert "/_matrix/media/v3/upload" in c["url"]
    assert "filename=x.txt" in c["url"]


def test_upload_media_size_cap_rejects(monkeypatch):
    a = _adapter(MATRIX_MAX_UPLOAD_BYTES="100")
    with pytest.raises(RuntimeError, match="exceeds 100 byte"):
        a._upload_media(b"x" * 200, "big", "application/octet-stream")


def test_upload_media_failure_raises(monkeypatch):
    fake = _FakeUrlopen([(413, {"errcode": "M_TOO_LARGE"})])
    monkeypatch.setattr(mx.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="status=413"):
        a._upload_media(b"x", "f", "text/plain")


# ---- _validate (whoami) ---------------------------------------------


def test_validate_returns_user_id(monkeypatch):
    fake = _FakeUrlopen([(200, {"user_id": "@bot:matrix.test"})])
    monkeypatch.setattr(mx.urllib.request, "urlopen", fake)
    a = _adapter()
    assert a._validate() == "@bot:matrix.test"
    c = fake.calls[0]
    assert c["method"] == "GET"
    assert c["url"].endswith("/_matrix/client/v3/account/whoami")


def test_validate_401_raises(monkeypatch):
    fake = _FakeUrlopen([(401, {"errcode": "M_UNKNOWN_TOKEN"})])
    monkeypatch.setattr(mx.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="status=401"):
        a._validate()


# ---- _format_with_button_hints --------------------------------------


def test_format_with_button_hints_empty():
    assert mx._format_with_button_hints("hi", []) == "hi"


def test_format_with_button_hints_single_row():
    out = mx._format_with_button_hints(
        "Pick:",
        [[{"label": "yes"}, {"label": "no"}]],
    )
    assert out == "Pick:\n[yes] [no]"


# ---- on_send (text path through executor) ---------------------------


def _send_cmd(channel_id="!r:m.test", text="hi", content=None,
              thread_id=None, user=None):
    from librefang.sidecar.protocol import Send
    return Send(channel_id, text, content, thread_id, user or {})


@pytest.mark.asyncio
async def test_on_send_text_path(monkeypatch):
    fake = _FakeUrlopen([(200, {"event_id": "$srv"})])
    monkeypatch.setattr(mx.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(text="hello", content={"Text": "hello"}))
    assert len(fake.calls) == 1
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["msgtype"] == "m.text"
    assert body["body"] == "hello"


@pytest.mark.asyncio
async def test_on_send_text_chunks_long_message(monkeypatch):
    monkeypatch.setattr(mx, "MAX_MESSAGE_LEN", 5)
    fake = _FakeUrlopen([
        (200, {"event_id": "$1"}),
        (200, {"event_id": "$2"}),
        (200, {"event_id": "$3"}),
    ])
    monkeypatch.setattr(mx.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(text="abcdefghijk", content={"Text": "abcdefghijk"}))
    assert len(fake.calls) == 3


@pytest.mark.asyncio
async def test_on_send_thread_wraps_relation(monkeypatch):
    fake = _FakeUrlopen([(200, {"event_id": "$srv"})])
    monkeypatch.setattr(mx.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(
        text="thread reply", content={"Text": "thread reply"},
        thread_id="$root",
    ))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["m.relates_to"]["rel_type"] == "m.thread"
    assert body["m.relates_to"]["event_id"] == "$root"


@pytest.mark.asyncio
async def test_on_send_empty_room_drops_silently(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(mx.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(channel_id="", user={}))
    assert fake.calls == []


@pytest.mark.asyncio
async def test_on_send_falls_back_to_user_platform_id(monkeypatch):
    fake = _FakeUrlopen([(200, {"event_id": "$srv"})])
    monkeypatch.setattr(mx.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(
        channel_id="",
        text="hi",
        content={"Text": "hi"},
        user={"platform_id": "!fallback:m.test"},
    ))
    c = fake.calls[0]
    assert "/rooms/" in c["url"]
    assert "%21fallback%3Am.test" in c["url"] or "!fallback" in c["url"]


# ---- schema / capability contract -----------------------------------


def test_schema_round_trip():
    schema = mx.MatrixAdapter.SCHEMA.to_dict()
    assert schema["name"] == "matrix"
    keys = {f["key"] for f in schema["fields"]}
    expected = {
        "MATRIX_HOMESERVER_URL",
        "MATRIX_USER_ID",
        "MATRIX_ACCESS_TOKEN",
        "MATRIX_ALLOWED_ROOMS",
        "MATRIX_ACCOUNT_ID",
        "MATRIX_MAX_UPLOAD_BYTES",
    }
    assert expected.issubset(keys), f"missing: {expected - keys}"
    secret_fields = {
        f["key"] for f in schema["fields"] if f["type"] == "secret"
    }
    assert secret_fields == {"MATRIX_ACCESS_TOKEN"}


def test_capabilities_declares_full_set():
    assert "thread" in mx.MatrixAdapter.capabilities
    assert "typing" in mx.MatrixAdapter.capabilities
    assert "reaction" in mx.MatrixAdapter.capabilities
    assert "streaming" in mx.MatrixAdapter.capabilities
