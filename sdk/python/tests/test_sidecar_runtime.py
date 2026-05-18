"""Runtime behaviour tests for librefang.sidecar.runtime.

Driven through the injectable I/O of `run()` (no real subprocess):
a queue feeds stdin lines, a list captures emitted events.
"""

import asyncio
import json

import pytest

from librefang.sidecar import ProducerCrashed, SidecarAdapter, run
from librefang.sidecar.protocol import Send


class RecordingAdapter(SidecarAdapter):
    capabilities = ["typing"]

    def __init__(self):
        self.sends = []
        self.commands = []
        self.shutdown_called = False

    async def on_send(self, cmd: Send) -> None:
        self.sends.append(cmd)

    async def on_command(self, cmd) -> None:
        self.commands.append(cmd)
        await super().on_command(cmd)

    async def on_shutdown(self) -> None:
        self.shutdown_called = True


async def _drive(adapter, lines, *, ready_interval=0.01, timeout=2.0):
    """Feed `lines` (then EOF) into run(); return emitted events."""
    q: asyncio.Queue = asyncio.Queue()
    for ln in lines:
        q.put_nowait(ln)
    q.put_nowait(None)  # EOF -> run() returns

    emitted = []

    async def line_source():
        return await q.get()

    await asyncio.wait_for(
        run(adapter, line_source=line_source, emit=emitted.append,
            ready_interval=ready_interval),
        timeout=timeout,
    )
    return emitted


async def test_ready_handshake_stops_after_ack():
    adapter = RecordingAdapter()
    # No ack -> several ready re-announces before EOF ends the run.
    emitted = await _drive(adapter, [], ready_interval=0.01)
    readies = [e for e in emitted if e["method"] == "ready"]
    assert len(readies) >= 1
    assert readies[0]["params"]["capabilities"] == ["typing"]

    # With an early ack, re-announcing stops; only the first ready or two.
    adapter2 = RecordingAdapter()
    emitted2 = await _drive(
        adapter2,
        ['{"method":"ready_ack"}'],
        ready_interval=0.5,  # long; ack must short-circuit the wait
    )
    assert sum(1 for e in emitted2 if e["method"] == "ready") <= 2


async def test_ready_reannounce_is_bounded_without_ack():
    # No ack ever arrives (pre-#5219 daemon). The loop must stop
    # re-announcing after ready_max_attempts instead of flooding
    # stdout forever, while the run keeps serving until shutdown.
    adapter = RecordingAdapter()
    emitted = []
    delivered = False

    async def line_source():
        nonlocal delivered
        if not delivered:
            delivered = True
            # > 3 * ready_interval, so the capped loop has finished.
            await asyncio.sleep(0.2)
            return '{"method":"shutdown"}'
        return None

    await asyncio.wait_for(
        run(adapter, line_source=line_source, emit=emitted.append,
            ready_interval=0.01, ready_max_attempts=3),
        timeout=2.0,
    )
    readies = [e for e in emitted if e["method"] == "ready"]
    assert len(readies) == 3  # capped, not unbounded
    assert adapter.shutdown_called  # run lifecycle still intact

    # ready_max_attempts=0 keeps the legacy unbounded behaviour.
    adapter2 = RecordingAdapter()
    emitted2 = []
    delivered2 = False

    async def line_source2():
        nonlocal delivered2
        if not delivered2:
            delivered2 = True
            await asyncio.sleep(0.1)
            return '{"method":"shutdown"}'
        return None

    await asyncio.wait_for(
        run(adapter2, line_source=line_source2, emit=emitted2.append,
            ready_interval=0.01, ready_max_attempts=0),
        timeout=2.0,
    )
    # ~0.1s / 0.01s interval ≫ 3: proves it did not self-cap.
    assert sum(1 for e in emitted2 if e["method"] == "ready") > 3


async def test_send_command_dispatched():
    adapter = RecordingAdapter()
    line = json.dumps({
        "method": "send",
        "params": {"channel_id": "c", "text": "hello",
                   "content": {"Text": "hello"},
                   "user": {"platform_id": "c", "display_name": "D",
                            "librefang_user": None}},
    })
    await _drive(adapter, ['{"method":"ready_ack"}', line])
    assert len(adapter.sends) == 1
    assert adapter.sends[0].text == "hello"
    assert adapter.sends[0].content == {"Text": "hello"}


async def test_unknown_command_does_not_crash():
    adapter = RecordingAdapter()
    await _drive(adapter, [
        '{"method":"ready_ack"}',
        '{"method":"some_future_cmd","params":{}}',
        '{"method":"send","params":{"channel_id":"c","text":"ok","user":{}}}',
    ])
    # Run survived the unknown command and still dispatched the send.
    assert any(s.text == "ok" for s in adapter.sends)


async def test_shutdown_command_ends_run_and_calls_hook():
    adapter = RecordingAdapter()
    # Shutdown before EOF; run() must return promptly and call on_shutdown.
    await _drive(adapter, ['{"method":"ready_ack"}', '{"method":"shutdown"}'])
    assert adapter.shutdown_called is True


async def test_invalid_json_emits_error_and_continues():
    adapter = RecordingAdapter()
    emitted = await _drive(adapter, [
        "not-json{",
        '{"method":"send","params":{"channel_id":"c","text":"after","user":{}}}',
    ])
    assert any(e["method"] == "error" for e in emitted)
    assert any(s.text == "after" for s in adapter.sends)


async def test_producer_emits_inbound_messages():
    class Producer(SidecarAdapter):
        async def on_send(self, cmd):  # unused here
            pass

        async def produce(self, emit):
            from librefang.sidecar import Content, protocol
            emit(protocol.message("u", "n", content=Content.text("tick")))
            # then idle until shutdown/EOF cancels us
            await asyncio.sleep(60)

    emitted = await _drive(Producer(), [], ready_interval=0.01)
    msgs = [e for e in emitted if e["method"] == "message"]
    assert msgs and msgs[0]["params"]["content"] == {"Text": "tick"}


@pytest.mark.parametrize("bad", ["", "   ", "\n"])
async def test_blank_lines_are_skipped(bad):
    adapter = RecordingAdapter()
    await _drive(adapter, [bad, '{"method":"shutdown"}'])
    assert adapter.shutdown_called


async def test_producer_crash_exits_nonzero_after_cleanup():
    # A fatal, unhandled producer error must surface as ProducerCrashed
    # (which run_stdio turns into a nonzero process exit, not a clean
    # shutdown), and on_shutdown cleanup must still run before it.
    class Crashing(SidecarAdapter):
        def __init__(self):
            self.shutdown_called = False

        async def on_send(self, cmd):
            pass

        async def produce(self, emit):
            raise RuntimeError("transport died unrecoverably")

        async def on_shutdown(self):
            self.shutdown_called = True

    adapter = Crashing()
    with pytest.raises(ProducerCrashed) as ei:
        await _drive(adapter, [])
    # __cause__ preserves the original transport error for diagnostics.
    assert isinstance(ei.value.__cause__, RuntimeError)
    assert "transport died unrecoverably" in str(ei.value.__cause__)
    assert adapter.shutdown_called, "cleanup must run before nonzero exit"


def test_run_stdio_translates_producer_crash_to_nonzero_exit():
    # run_stdio is the process entry point: a ProducerCrashed from run()
    # must become SystemExit(1) so the daemon supervisor sees a nonzero
    # exit, distinguishable from a clean shutdown/EOF. Exercised
    # synchronously because run_stdio owns its own event loop.
    from librefang.sidecar import run_stdio

    class Crashing(SidecarAdapter):
        async def on_send(self, cmd):
            pass

        async def produce(self, emit):
            raise RuntimeError("transport died unrecoverably")

    # Closed stdin -> reader thread hits EOF immediately; the producer
    # crash still wins the race because `stop` is set before EOF reaches
    # the loop. ready_max_attempts=1 caps the ready-loop quickly.
    import io
    import sys
    saved_stdin, saved_stdout = sys.stdin, sys.stdout
    sys.stdin = io.StringIO("")
    sys.stdout = io.StringIO()
    try:
        with pytest.raises(SystemExit) as ei:
            run_stdio(Crashing(), ready_interval=0.01, ready_max_attempts=1)
        assert ei.value.code == 1
    finally:
        sys.stdin, sys.stdout = saved_stdin, saved_stdout
