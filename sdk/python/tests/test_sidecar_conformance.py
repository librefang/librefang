"""Python side of the sidecar protocol conformance suite.

The shared corpus (``conformance/sidecar/corpus/``) is the single
oracle for the protocol's two implementations — this SDK's
``protocol.py`` and the Rust supervisor's ``SidecarEvent`` /
``SidecarCommand``. The Rust half lives in
``crates/librefang-channels/tests/sidecar_protocol_conformance.rs``
and asserts against the same files.

Directionality (see ``conformance/sidecar/README.md``):
* **commands** are produced by the supervisor and *consumed* here —
  the SDK is the parser, so we assert ``parse_command`` on each corpus
  command yields the expected typed object.
* **events** are *produced* here — the SDK is the serializer, so we
  assert each builder reproduces the corpus frame.

Equality is structural JSON value equality, not byte equality.
"""

import json
from pathlib import Path

import pytest

from librefang.sidecar import protocol
from librefang.sidecar.protocol import (
    Content,
    Heartbeat,
    Interactive,
    Reaction,
    ReadyAck,
    Send,
    Shutdown,
    StreamDelta,
    StreamEnd,
    StreamStart,
    TypingCmd,
    parse_command,
)

CORPUS = Path(__file__).resolve().parents[3] / "conformance" / "sidecar" / "corpus"

# The bare legacy ``{"method":"ready"}`` form: the SDK never *emits*
# it (its ready() builder always writes full params); it exists so the
# Rust consumer's backward-compat acceptance stays pinned. Documented
# producer-side skip — asserted on the Rust side instead.
EVENT_PRODUCER_SKIP = {"ready_minimal.json"}


def _corpus(rel: str) -> dict:
    return json.loads((CORPUS / rel).read_text(encoding="utf-8"))


def _list(subdir: str) -> set:
    d = CORPUS / subdir
    return {p.name for p in d.glob("*.json")}


def test_corpus_present():
    assert CORPUS.is_dir(), f"corpus dir missing: {CORPUS}"
    assert _list("events"), "no event fixtures"
    assert _list("commands"), "no command fixtures"


# ---- events: producer side (builder output == corpus) ----------------

# Each event fixture mapped to the builder call that must reproduce it.
EVENT_BUILDERS = {
    "ready_full.json": lambda: protocol.ready(
        capabilities=["typing", "reaction", "interactive", "thread",
                      "streaming"],
        account_id="bot-1",
        protocol_version=1,
    ),
    "message_text.json": lambda: protocol.message(
        "42", "Alice", content=Content.text("hello"),
        channel_id="-100123", platform="telegram",
    ),
    "message_minimal.json": lambda: protocol.message("1", "Bob", text="hi"),
    "error.json": lambda: protocol.error("boom"),
    "typing.json": lambda: protocol.typing_event("u", "n", True),
}


def test_every_event_fixture_is_covered():
    # A fixture with neither a producer assertion nor a documented skip
    # is not conformance — mirror the Rust coverage guard.
    assert set(EVENT_BUILDERS) | EVENT_PRODUCER_SKIP == _list("events")


@pytest.mark.parametrize("name", sorted(EVENT_BUILDERS))
def test_event_builder_matches_corpus(name):
    assert EVENT_BUILDERS[name]() == _corpus(f"events/{name}")


# ---- commands: consumer side (parse_command(corpus) == expected) -----


def _parse(name: str):
    return parse_command(json.dumps(_corpus(f"commands/{name}")))


def test_every_command_fixture_is_covered():
    asserted = {
        "send_full.json", "send_minimal.json", "ready_ack.json",
        "shutdown.json", "heartbeat.json", "typing.json", "reaction.json",
        "interactive.json", "stream_start.json",
        "stream_start_threaded.json", "stream_delta.json",
        "stream_end.json",
    }
    assert asserted == _list("commands")


def test_command_send_full():
    c = _parse("send_full.json")
    assert isinstance(c, Send)
    assert c.channel_id == "c1" and c.text == "hello"
    assert c.content == {"Text": "hello"} and c.thread_id == "t1"
    assert c.user == {"platform_id": "c1", "display_name": "Alice",
                      "librefang_user": None}


def test_command_send_minimal():
    c = _parse("send_minimal.json")
    assert isinstance(c, Send)
    assert c.channel_id == "c1" and c.text == "hi"
    assert c.content is None and c.thread_id is None


def test_command_parameterless():
    assert isinstance(_parse("ready_ack.json"), ReadyAck)
    assert isinstance(_parse("shutdown.json"), Shutdown)
    assert isinstance(_parse("heartbeat.json"), Heartbeat)


def test_command_typing():
    c = _parse("typing.json")
    assert isinstance(c, TypingCmd) and c.channel_id == "c1"


def test_command_reaction():
    c = _parse("reaction.json")
    assert isinstance(c, Reaction)
    assert c.channel_id == "c1" and c.message_id == "55"
    assert c.reaction == "👍"


def test_command_interactive():
    c = _parse("interactive.json")
    assert isinstance(c, Interactive)
    assert c.channel_id == "c1"
    assert c.message == {
        "text": "pick",
        "buttons": [[
            {"label": "Yes", "action": "y"},
            {"label": "Docs", "action": "d", "url": "https://x"},
        ]],
    }


def test_command_stream_start_and_threaded():
    c = _parse("stream_start.json")
    assert isinstance(c, StreamStart)
    assert c.channel_id == "c1" and c.stream_id == "s1"
    assert c.thread_id is None
    t = _parse("stream_start_threaded.json")
    assert isinstance(t, StreamStart) and t.thread_id == "t1"


def test_command_stream_delta_end():
    d = _parse("stream_delta.json")
    assert isinstance(d, StreamDelta)
    assert d.stream_id == "s1" and d.text == "Hel"
    e = _parse("stream_end.json")
    assert isinstance(e, StreamEnd) and e.stream_id == "s1"
