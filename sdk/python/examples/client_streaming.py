#!/usr/bin/env python3
"""Stream an agent response through the LibreFang REST API."""

import sys
from typing import Any, Optional

from librefang import Client
from librefang.librefang_client import LibreFangError


def main(client: Optional[Any] = None) -> int:
    client = client or Client("http://localhost:4545")
    agent_id = None
    failed = False

    try:
        agent = client.agents.spawn_agent(template="assistant")
        agent_id = agent["agent_id"]
        print(f"Agent: {agent_id}")

        print("\n--- Streaming response ---")
        events = client.agents.send_message_stream(
            agent_id,
            message="Tell me a short story about a robot.",
        )
        for event in events:
            if "error" in event or event.get("raw") == "error":
                detail = event.get("error", event.get("raw"))
                raise LibreFangError(f"Stream error: {detail}")
            if event.get("content"):
                print(event["content"], end="", flush=True)
            elif event.get("tool"):
                print(f"\n[Tool call: {event['tool']}]")
            elif event.get("done"):
                print("\n--- Done ---")
    except LibreFangError as error:
        print(f"LibreFang request failed: {error}", file=sys.stderr)
        failed = True
    finally:
        if agent_id is not None:
            try:
                client.agents.kill_agent(agent_id, confirm=True)
                print("Agent deleted.")
            except LibreFangError as error:
                print(f"Agent cleanup failed: {error}", file=sys.stderr)
                failed = True

    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
