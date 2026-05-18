"""Wire-shape tests for librefang.sidecar.protocol.

These assert the JSON the SDK emits/parses matches the Rust
SidecarEvent/SidecarCommand + ChannelContent shapes byte-for-byte
(externally-tagged enum: {"Text": "..."} / {"Image": {...}}).
"""

import json

import pytest

from librefang.sidecar import protocol
from librefang.sidecar.protocol import (
    Content,
    Heartbeat,
    Interactive,
    ReadyAck,
    Reaction,
    Send,
    Shutdown,
    StreamDelta,
    StreamEnd,
    StreamStart,
    TypingCmd,
    UnknownCommand,
    parse_command,
)


def test_content_variants_are_externally_tagged():
    assert Content.text("hi") == {"Text": "hi"}
    assert Content.image("u", "c", "image/png") == {
        "Image": {"url": "u", "caption": "c", "mime_type": "image/png"}
    }
    assert Content.file("u", "f.pdf") == {
        "File": {"url": "u", "filename": "f.pdf"}
    }
    # FileData.data is a JSON number array (mirrors Rust Vec<u8>).
    assert Content.file_data(b"\x01\x02", "b.bin", "application/octet-stream") == {
        "FileData": {
            "data": [1, 2],
            "filename": "b.bin",
            "mime_type": "application/octet-stream",
        }
    }
    assert Content.location(1.5, -2.0) == {"Location": {"lat": 1.5, "lon": -2.0}}
    assert Content.command("start", ["a"]) == {
        "Command": {"name": "start", "args": ["a"]}
    }
    assert Content.sticker("s1") == {"Sticker": {"file_id": "s1"}}
    assert Content.poll_answer("p", [0, 1]) == {
        "PollAnswer": {"poll_id": "p", "option_ids": [0, 1]}
    }
    btn = Content.button("Yes", "yes", style="primary")
    assert btn == {"label": "Yes", "action": "yes", "style": "primary"}
    assert Content.interactive("pick", [[btn]]) == {
        "Interactive": {"text": "pick", "buttons": [[btn]]}
    }


def test_ready_event_full_and_minimal():
    ev = protocol.ready(capabilities=["typing", "streaming"],
                        account_id="bot-1", protocol_version=1)
    assert ev["method"] == "ready"
    p = ev["params"]
    assert p["capabilities"] == ["typing", "streaming"]
    assert p["account_id"] == "bot-1"
    assert p["protocol_version"] == 1
    assert p["suppress_error_responses"] is False
    assert p["notification_recipients"] == []
    assert p["header_rules"] == []
    # Round-trips through JSON unchanged.
    assert json.loads(json.dumps(ev)) == ev

    bare = protocol.ready()
    assert bare["params"]["capabilities"] == []


def test_message_event_omits_unset_optionals():
    ev = protocol.message("u1", "Alice", content=Content.text("hi"),
                          channel_id="c", is_group=True)
    p = ev["params"]
    assert ev["method"] == "message"
    assert p["user_id"] == "u1" and p["user_name"] == "Alice"
    assert p["content"] == {"Text": "hi"}
    assert p["channel_id"] == "c"
    assert p["is_group"] is True
    # Unset optionals are not serialised (Rust uses #[serde(default)]).
    assert "thread_id" not in p
    assert "metadata" not in p
    assert "platform" not in p


def test_text_content_mirrors_legacy_text_field():
    # content-only plain text also populates legacy `text` so a
    # pre-#5219 daemon (ignores `content`) doesn't get an empty message.
    p = protocol.message("u", "n", content=Content.text("hi"))["params"]
    assert p["content"] == {"Text": "hi"}
    assert p["text"] == "hi"

    # Non-text content can't be flattened — no spurious `text`.
    p2 = protocol.message(
        "u", "n", content=Content.image("https://x/i.png"))["params"]
    assert "Image" in p2["content"]
    assert "text" not in p2

    # Explicit text is never overwritten by the mirror.
    p3 = protocol.message(
        "u", "n", text="explicit", content=Content.text("hi"))["params"]
    assert p3["text"] == "explicit"


def test_error_and_typing_events():
    assert protocol.error("boom") == {
        "method": "error", "params": {"message": "boom"}
    }
    assert protocol.typing_event("u", "n", True) == {
        "method": "typing",
        "params": {"user_id": "u", "user_name": "n", "is_typing": True},
    }


def test_parse_command_all_kinds():
    send = parse_command(json.dumps({
        "method": "send",
        "params": {
            "channel_id": "c", "text": "hi",
            "content": {"Text": "hi"}, "thread_id": "t1",
            "user": {"platform_id": "c", "display_name": "D",
                     "librefang_user": None},
        },
    }))
    assert isinstance(send, Send)
    assert send.channel_id == "c" and send.text == "hi"
    assert send.content == {"Text": "hi"} and send.thread_id == "t1"
    assert send.user["platform_id"] == "c"

    assert isinstance(parse_command('{"method":"ready_ack"}'), ReadyAck)
    assert isinstance(parse_command('{"method":"shutdown"}'), Shutdown)
    assert isinstance(parse_command('{"method":"heartbeat"}'), Heartbeat)

    t = parse_command('{"method":"typing","params":{"channel_id":"c"}}')
    assert isinstance(t, TypingCmd) and t.channel_id == "c"

    r = parse_command(json.dumps({
        "method": "reaction",
        "params": {"channel_id": "c", "message_id": "m", "reaction": "👍"},
    }))
    assert isinstance(r, Reaction) and r.reaction == "👍"

    i = parse_command(json.dumps({
        "method": "interactive",
        "params": {"channel_id": "c", "message": {"text": "x", "buttons": []}},
    }))
    assert isinstance(i, Interactive) and i.message["text"] == "x"

    ss = parse_command(json.dumps({
        "method": "stream_start",
        "params": {"channel_id": "c", "stream_id": "s"},
    }))
    assert isinstance(ss, StreamStart) and ss.stream_id == "s"
    sd = parse_command(json.dumps({
        "method": "stream_delta", "params": {"stream_id": "s", "text": "ab"},
    }))
    assert isinstance(sd, StreamDelta) and sd.text == "ab"
    se = parse_command('{"method":"stream_end","params":{"stream_id":"s"}}')
    assert isinstance(se, StreamEnd) and se.stream_id == "s"


def test_parse_command_unknown_is_tolerated():
    # Forward-compat: a future command must not crash the parser.
    u = parse_command('{"method":"some_future_thing","params":{"x":1}}')
    assert isinstance(u, UnknownCommand)
    assert u.method == "some_future_thing"
    assert u.raw["params"] == {"x": 1}


def test_parse_command_non_object_json_raises_decode_error():
    # Syntactically valid JSON that is not an object must surface as a
    # JSONDecodeError (the reader's existing handled path), not an
    # AttributeError that escapes and wedges the adapter.
    for bad in ("123", '"a string"', "[1,2,3]", "true", "null"):
        with pytest.raises(json.JSONDecodeError):
            parse_command(bad)


def test_parse_command_stream_start_carries_thread_id():
    ss = parse_command(json.dumps({
        "method": "stream_start",
        "params": {"channel_id": "c", "stream_id": "s", "thread_id": "t-1"},
    }))
    assert isinstance(ss, StreamStart)
    assert ss.thread_id == "t-1"
    # Absent → None (pre-thread wire shape).
    ss2 = parse_command(
        '{"method":"stream_start","params":{"channel_id":"c","stream_id":"s"}}')
    assert isinstance(ss2, StreamStart) and ss2.thread_id is None


def test_message_carries_platform_message_id():
    p = protocol.message("u", "n", text="hi", message_id="plat-7")["params"]
    assert p["message_id"] == "plat-7"
    # Absent → omitted (server generates a UUID).
    p2 = protocol.message("u", "n", text="hi")["params"]
    assert "message_id" not in p2
