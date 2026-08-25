"""Runtime behaviour tests for librefang.sidecar.runtime.

Driven through the injectable I/O of `run()` (no real subprocess):
a queue feeds stdin lines, a list captures emitted events.
"""

import asyncio
import io
import json

import pytest

from librefang.sidecar import ProducerCrashed, ReaderCrashed, SidecarAdapter, run
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
    ready_cap_reached = asyncio.Event()
    delivered = False

    def emit(event):
        emitted.append(event)
        if event["method"] == "ready":
            readies = sum(item["method"] == "ready" for item in emitted)
            if readies == 3:
                ready_cap_reached.set()

    async def line_source():
        nonlocal delivered
        if not delivered:
            delivered = True
            await ready_cap_reached.wait()
            return '{"method":"shutdown"}'
        return None

    await asyncio.wait_for(
        run(adapter, line_source=line_source, emit=emit,
            ready_interval=0.01, ready_max_attempts=3),
        timeout=2.0,
    )
    readies = [e for e in emitted if e["method"] == "ready"]
    assert len(readies) == 3  # capped, not unbounded
    assert adapter.shutdown_called  # run lifecycle still intact

    # ready_max_attempts=0 keeps the legacy unbounded behaviour.
    adapter2 = RecordingAdapter()
    emitted2 = []
    fourth_ready_emitted = asyncio.Event()
    delivered2 = False

    def emit2(event):
        emitted2.append(event)
        if event["method"] == "ready":
            readies = sum(item["method"] == "ready" for item in emitted2)
            if readies == 4:
                fourth_ready_emitted.set()

    async def line_source2():
        nonlocal delivered2
        if not delivered2:
            delivered2 = True
            await fourth_ready_emitted.wait()
            return '{"method":"shutdown"}'
        return None

    await asyncio.wait_for(
        run(adapter2, line_source=line_source2, emit=emit2,
            ready_interval=0.01, ready_max_attempts=0),
        timeout=2.0,
    )
    assert sum(1 for e in emitted2 if e["method"] == "ready") >= 4


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


async def test_non_object_command_params_emit_error_and_continue():
    adapter = RecordingAdapter()
    emitted = await _drive(adapter, [
        '{"method":"send","params":["not","an","object"]}',
        '{"method":"send","params":{"channel_id":"c","text":"after","user":{}}}',
    ])

    assert any(e["method"] == "error" for e in emitted)
    assert [send.text for send in adapter.sends] == ["after"]


async def test_non_string_command_method_emits_error_and_continues():
    adapter = RecordingAdapter()
    emitted = await _drive(adapter, [
        '{"method":[],"params":{}}',
        '{"method":"send","params":{"channel_id":"c","text":"after","user":{}}}',
    ])

    assert any(e["method"] == "error" for e in emitted)
    assert [send.text for send in adapter.sends] == ["after"]


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


async def test_reader_crash_stops_run_after_cleanup():
    adapter = RecordingAdapter()

    async def broken_line_source():
        raise RuntimeError("stdin transport failed")

    with pytest.raises(ReaderCrashed) as error:
        await asyncio.wait_for(
            run(
                adapter,
                line_source=broken_line_source,
                emit=lambda _event: None,
                ready_interval=0.01,
            ),
            timeout=1.0,
        )

    assert isinstance(error.value.__cause__, RuntimeError)
    assert "stdin transport failed" in str(error.value.__cause__)
    assert adapter.shutdown_called, "cleanup must run before reader failure surfaces"


async def test_unexpected_parser_error_stops_reader_instead_of_hanging(monkeypatch):
    adapter = RecordingAdapter()
    delivered = False

    async def line_source():
        nonlocal delivered
        if not delivered:
            delivered = True
            return '{"method":"send","params":{}}'
        return None

    def broken_parser(_line):
        raise TypeError("unexpected parser failure")

    monkeypatch.setattr("librefang.sidecar.runtime.protocol.parse_command", broken_parser)

    with pytest.raises(ReaderCrashed) as error:
        await asyncio.wait_for(
            run(
                adapter,
                line_source=line_source,
                emit=lambda _event: None,
                ready_interval=0.01,
            ),
            timeout=1.0,
        )

    assert isinstance(error.value.__cause__, TypeError)
    assert adapter.shutdown_called


async def test_reader_emit_failure_stops_run_after_cleanup():
    adapter = RecordingAdapter()
    lines = iter(["not-json{", None])

    async def line_source():
        return next(lines)

    def emit(event):
        if event["method"] == "error":
            raise OSError("stdout write failed")

    with pytest.raises(ReaderCrashed) as error:
        await asyncio.wait_for(
            run(
                adapter,
                line_source=line_source,
                emit=emit,
                ready_interval=0.01,
            ),
            timeout=1.0,
        )

    assert isinstance(error.value.__cause__, OSError)
    assert adapter.shutdown_called


async def test_stdio_reader_thread_failure_reaches_async_reader(monkeypatch):
    from librefang.sidecar.runtime import _run_stdio

    class BrokenStdin:
        def __iter__(self):
            return self

        def __next__(self):
            raise OSError("stdin device failed")

    adapter = RecordingAdapter()
    monkeypatch.setattr("sys.stdin", BrokenStdin())
    monkeypatch.setattr("sys.stdout", io.StringIO())

    with pytest.raises(ReaderCrashed) as error:
        await asyncio.wait_for(
            _run_stdio(
                adapter,
                ready_interval=0.01,
                ready_max_attempts=1,
            ),
            timeout=1.0,
        )

    assert isinstance(error.value.__cause__, OSError)
    assert "stdin device failed" in str(error.value.__cause__)
    assert adapter.shutdown_called


def test_run_stdio_translates_producer_crash_to_nonzero_exit(monkeypatch):
    # run_stdio is the process entry point: a ProducerCrashed from run()
    # must become SystemExit(1) so the daemon supervisor sees a nonzero
    # exit, distinguishable from a clean shutdown/EOF. Exercised
    # synchronously because run_stdio owns its own event loop.
    from librefang.sidecar import run_stdio

    async def crash(*_args, **_kwargs):
        raise ProducerCrashed("producer failed")

    monkeypatch.setattr("librefang.sidecar.runtime._run_stdio", crash)

    with pytest.raises(SystemExit) as error:
        run_stdio(RecordingAdapter(), ready_interval=0.01, ready_max_attempts=1)

    assert error.value.code == 1
