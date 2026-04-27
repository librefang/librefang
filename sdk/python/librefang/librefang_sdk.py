"""
LibreFang Python SDK — helper library for writing Python agents.

Usage:

    from librefang_sdk import Agent

    agent = Agent()

    @agent.on_message
    def handle(message: str, context: dict) -> str:
        return f"You said: {message}"

    agent.run()

Or for simple scripts without the decorator pattern:

    from librefang_sdk import read_input, respond

    data = read_input()
    result = f"Echo: {data['message']}"
    respond(result)

Live progress logging
---------------------
The LibreFang daemon streams every line your script writes to stderr to
its own tracing subsystem under the ``python_stderr`` target — visible
with ``RUST_LOG=python_stderr=info`` (or filtered through ``journalctl``
/ ``docker logs`` when the daemon runs as a service). This is the
recommended channel for "still working" / progress heartbeats from
long-running tools.

**Python buffers stderr by default**, so for the daemon to actually
see lines as they happen, either:

- pass ``flush=True`` to ``print(..., file=sys.stderr, flush=True)``,
- run the interpreter line-buffered with ``python -u``, or
- call ``sys.stderr.reconfigure(line_buffering=True)`` once at startup.

Without one of these, stderr is block-buffered (~4–8 KiB) and the
daemon will only log lines once the buffer fills or the process exits,
defeating the purpose of live streaming.
"""

import json
import os
import sys
from typing import Callable, Optional, Dict, Any


def read_input() -> Dict[str, Any]:
    """Read the input JSON from stdin (sent by the LibreFang kernel)."""
    line = sys.stdin.readline().strip()
    if not line:
        # Fallback: check environment variables
        agent_id = os.environ.get("LIBREFANG_AGENT_ID", "")
        message = os.environ.get("LIBREFANG_MESSAGE", "")
        return {
            "type": "message",
            "agent_id": agent_id,
            "message": message,
            "context": {},
        }
    return json.loads(line)


def respond(text: str, metadata: Optional[Dict[str, Any]] = None) -> None:
    """Send a response back to the LibreFang kernel via stdout."""
    response = {"type": "response", "text": text}
    if metadata:
        response["metadata"] = metadata
    print(json.dumps(response), flush=True)


def log(message: str, level: str = "info") -> None:
    """Log a message to stderr (visible in LibreFang daemon logs)."""
    print(f"[{level.upper()}] {message}", file=sys.stderr, flush=True)


class Agent:
    """Decorator-based Python agent framework.

    Example:

        agent = Agent()

        @agent.on_message
        def handle(message: str, context: dict) -> str:
            return f"Hello! You said: {message}"

        agent.run()
    """

    def __init__(self):
        self._handler: Optional[Callable] = None
        self._setup: Optional[Callable] = None
        self._teardown: Optional[Callable] = None

    def on_message(self, func: Callable) -> Callable:
        """Register a message handler function.

        The function should accept (message: str, context: dict) and return str.
        """
        self._handler = func
        return func

    def on_setup(self, func: Callable) -> Callable:
        """Register a setup function called once before message handling."""
        self._setup = func
        return func

    def on_teardown(self, func: Callable) -> Callable:
        """Register a teardown function called once after message handling."""
        self._teardown = func
        return func

    def run(self) -> None:
        """Run the agent, reading input and producing output."""
        if self._handler is None:
            log("No message handler registered", "error")
            sys.exit(1)

        try:
            if self._setup:
                self._setup()

            data = read_input()
            message = data.get("message", "")
            context = data.get("context", {})

            result = self._handler(message, context)

            if isinstance(result, str):
                respond(result)
            elif isinstance(result, dict):
                respond(result.get("text", str(result)), result.get("metadata"))
            else:
                respond(str(result))

        except Exception as e:
            log(f"Agent error: {e}", "error")
            respond(f"Error: {e}")
            sys.exit(1)
        finally:
            if self._teardown:
                try:
                    self._teardown()
                except Exception as e:
                    log(f"Teardown error: {e}", "error")


# Convenience: if this file is run directly, show usage
if __name__ == "__main__":
    print("LibreFang Python SDK")
    print("====================")
    print()
    print("Import this module in your agent scripts:")
    print()
    print("  from librefang_sdk import Agent")
    print()
    print("  agent = Agent()")
    print()
    print("  @agent.on_message")
    print("  def handle(message, context):")
    print("      return f'You said: {message}'")
    print()
    print("  agent.run()")
