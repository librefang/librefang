"""LibreFang sidecar channel adapter SDK.

Write a channel adapter in Python that runs as a supervised subprocess
of LibreFang, speaking newline-delimited JSON-RPC over stdio:

    from librefang.sidecar import SidecarAdapter, run_stdio, Content, protocol

    class MyAdapter(SidecarAdapter):
        capabilities = ["typing"]

        async def on_send(self, cmd):
            ...  # deliver cmd.text / cmd.content to your platform

        async def produce(self, emit):
            async for m in my_platform_stream():
                emit(protocol.message(m.user_id, m.user_name,
                                      content=Content.text(m.text)))

    if __name__ == "__main__":
        run_stdio(MyAdapter())

See :mod:`librefang.sidecar.runtime` for the daemon-restart vs.
platform-reconnect responsibility split.
"""

from __future__ import annotations

from . import logging, protocol
from .protocol import (
    Command,
    Content,
    Field,
    Heartbeat,
    Interactive,
    ReadyAck,
    Reaction,
    Schema,
    Send,
    Shutdown,
    StreamDelta,
    StreamEnd,
    StreamStart,
    TypingCmd,
    UnknownCommand,
    parse_command,
)
from .runtime import (
    ProducerCrashed,
    SidecarAdapter,
    describe_main,
    run,
    run_stdio,
    with_backoff,
)

__all__ = [
    "ProducerCrashed",
    "SidecarAdapter",
    "describe_main",
    "run",
    "run_stdio",
    "with_backoff",
    "Content",
    "Field",
    "Schema",
    "protocol",
    "logging",
    "parse_command",
    "Command",
    "Send",
    "ReadyAck",
    "Shutdown",
    "TypingCmd",
    "Reaction",
    "Interactive",
    "StreamStart",
    "StreamDelta",
    "StreamEnd",
    "Heartbeat",
    "UnknownCommand",
]
