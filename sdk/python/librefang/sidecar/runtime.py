"""Runtime for LibreFang sidecar channel adapters.

Subclass :class:`SidecarAdapter`, implement :meth:`SidecarAdapter.on_send`
(and optionally :meth:`SidecarAdapter.produce` to push inbound platform
messages), then ``run_stdio(MyAdapter())``. The framework owns:

* the stdin command-reader loop and JSON parsing;
* the ``ready`` handshake — it re-announces ``ready`` (with your
  declared capabilities) every ``ready_interval`` until LibreFang sends
  ``ready_ack``, so a fresh process is idempotently re-discovered;
* graceful ``shutdown`` (runs :meth:`SidecarAdapter.on_shutdown`);
* keeping **stdout** free of anything but protocol frames (log via
  :mod:`librefang.sidecar.logging`, which writes stderr).

The ``ready`` → ``ready_ack`` handshake assumes the **post-#5219**
sidecar daemon (see :mod:`librefang.sidecar.protocol`). ``main``'s
daemon has no ``ready_ack`` command, so against a pre-#5219 daemon the
re-announce loop never terminates — this SDK is wire-complete ahead of
#5219 landing, not a drop-in for the current ``main`` protocol.

Responsibility split — read this:

* **Process restart is the daemon's job.** LibreFang's supervisor
  respawns a crashed sidecar with backoff and a circuit-breaker. Your
  adapter must be *crash-safe*: hold no irreplaceable in-process state,
  and let the framework re-announce ``ready`` on each fresh start. Do
  **not** try to keep your own process alive across a fatal error.
* **Platform reconnect is the adapter's job.** Reconnecting a dropped
  Telegram long-poll / WebSocket is your transport's concern. Use
  :func:`with_backoff` for that loop; it is independent of the
  daemon-managed process lifecycle.
"""

from __future__ import annotations

import asyncio
import json
import sys
import threading
from typing import Any, Awaitable, Callable, Dict, List, Optional

from . import logging as log
from . import protocol
from .protocol import (
    Command,
    ReadyAck,
    Send,
    Shutdown,
)

EmitFn = Callable[[Dict[str, Any]], None]
LineSource = Callable[[], Awaitable[Optional[str]]]


class SidecarAdapter:
    """Base class for a sidecar channel adapter.

    Override :meth:`on_send` (required) and, for platforms you poll,
    :meth:`produce`. Declare optional capabilities so LibreFang routes
    rich features (typing/reaction/interactive/thread/streaming/
    typing_events) to you instead of degrading to plain text.
    """

    #: Capability strings, e.g. ``["typing", "interactive"]``.
    capabilities: List[str] = []
    #: Multi-bot account id, if this adapter is one of several.
    account_id: Optional[str] = None
    #: Post errors to the user privately (log-only) when True.
    suppress_error_responses: bool = False
    #: Operator inbox(es) for non-conversational broadcasts.
    notification_recipients: List[Dict[str, Any]] = []
    #: ``[(host, [[k, v], ...]), ...]`` auth headers for media fetch.
    header_rules: List[Any] = []
    #: Optional protocol version (diagnostics only).
    protocol_version: Optional[int] = None

    def ready_event(self) -> Dict[str, Any]:
        return protocol.ready(
            self.capabilities,
            self.account_id,
            self.suppress_error_responses,
            self.notification_recipients,
            self.header_rules,
            self.protocol_version,
        )

    async def on_send(self, cmd: Send) -> None:
        """Deliver an outbound message to the platform. Required."""
        raise NotImplementedError(
            "override on_send() to deliver messages to your platform"
        )

    async def on_command(self, cmd: Command) -> None:
        """Dispatch a command. Default routes ``send`` to
        :meth:`on_send`; typing/reaction/interactive/stream_* are no-ops
        unless you override this. ``ready_ack``/``shutdown`` are handled
        by the framework and never reach here."""
        if isinstance(cmd, Send):
            await self.on_send(cmd)

    async def produce(self, emit: EmitFn) -> None:
        """Optional: pull inbound platform events and ``emit(event)``
        (build events with :mod:`librefang.sidecar.protocol`). Default:
        nothing — for command/webhook-only adapters."""
        return

    async def on_shutdown(self) -> None:
        """Optional cleanup on a clean shutdown."""
        return


async def with_backoff(
    fn: Callable[[], Awaitable[None]],
    *,
    initial: float = 1.0,
    maximum: float = 30.0,
    factor: float = 2.0,
) -> None:
    """Retry ``fn`` with exponential backoff until it returns without
    raising. For *platform* reconnection only — process restart is the
    daemon's job (see module docstring). Propagates cancellation."""
    delay = initial
    while True:
        try:
            await fn()
            return
        except asyncio.CancelledError:
            raise
        except Exception as e:  # noqa: BLE001 - transport errors vary
            log.warn("operation failed; backing off",
                     error=str(e), delay=delay)
            await asyncio.sleep(delay)
            delay = min(delay * factor, maximum)


async def run(
    adapter: SidecarAdapter,
    *,
    line_source: LineSource,
    emit: EmitFn,
    ready_interval: float = 2.0,
) -> None:
    """Drive an adapter against injectable I/O. ``line_source`` returns
    the next stdin line or ``None`` at EOF; ``emit`` writes one event.
    Returns when LibreFang sends ``shutdown`` or stdin reaches EOF."""
    acked = asyncio.Event()
    stop = asyncio.Event()

    async def ready_loop() -> None:
        while not acked.is_set() and not stop.is_set():
            emit(adapter.ready_event())
            try:
                await asyncio.wait_for(acked.wait(), timeout=ready_interval)
            except asyncio.TimeoutError:
                pass  # re-announce; ack is idempotent

    async def producer() -> None:
        try:
            await adapter.produce(emit)
        except asyncio.CancelledError:
            raise
        except Exception as e:  # noqa: BLE001
            log.error("producer crashed", error=str(e))
            stop.set()

    async def reader() -> None:
        while not stop.is_set():
            line = await line_source()
            if line is None:
                stop.set()
                return
            line = line.strip()
            if not line:
                continue
            try:
                cmd = protocol.parse_command(line)
            except json.JSONDecodeError as e:
                emit(protocol.error(f"invalid JSON: {e}"))
                continue
            if isinstance(cmd, ReadyAck):
                acked.set()
                continue
            if isinstance(cmd, Shutdown):
                stop.set()
                return
            try:
                await adapter.on_command(cmd)
            except Exception as e:  # noqa: BLE001
                log.error("on_command failed", error=str(e))

    tasks = [
        asyncio.ensure_future(ready_loop()),
        asyncio.ensure_future(producer()),
        asyncio.ensure_future(reader()),
    ]
    try:
        await stop.wait()
    finally:
        for t in tasks:
            t.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        try:
            await adapter.on_shutdown()
        except Exception as e:  # noqa: BLE001
            log.error("on_shutdown failed", error=str(e))


def run_stdio(adapter: SidecarAdapter, *, ready_interval: float = 2.0) -> None:
    """Wire ``run`` to real stdio and run it to completion. stdout is
    written under a lock and carries only protocol frames; stdin is read
    on a daemon thread (portable, unlike async stdin)."""
    asyncio.run(_run_stdio(adapter, ready_interval=ready_interval))


async def _run_stdio(adapter: SidecarAdapter, *,
                     ready_interval: float) -> None:
    loop = asyncio.get_event_loop()
    queue: "asyncio.Queue[Optional[str]]" = asyncio.Queue()

    def _reader_thread() -> None:
        for line in sys.stdin:
            loop.call_soon_threadsafe(queue.put_nowait, line)
        loop.call_soon_threadsafe(queue.put_nowait, None)

    threading.Thread(target=_reader_thread, daemon=True).start()

    async def line_source() -> Optional[str]:
        return await queue.get()

    write_lock = threading.Lock()

    def emit(event: Dict[str, Any]) -> None:
        data = json.dumps(event) + "\n"
        with write_lock:
            sys.stdout.write(data)
            sys.stdout.flush()

    await run(adapter, line_source=line_source, emit=emit,
              ready_interval=ready_interval)
