#!/usr/bin/env python3
"""Create an agent and chat with it through the LibreFang REST API."""

import sys
import time
from typing import Any, Optional

from librefang import Client
from librefang.librefang_client import LibreFangError


def main(client: Optional[Any] = None) -> int:
    client = client or Client("http://localhost:4545")
    created_agent_id = None
    failed = False

    try:
        print("Server:", client.system.health())

        page = client.agents.list_agents()
        agents = page.get("items", [])
        print(f"Agents: {page.get('total', len(agents))}")

        if agents:
            agent_id = agents[0]["id"]
            print(f"Using existing agent: {agent_id}")
        else:
            timestamp = int(time.time())
            agent = client.agents.spawn_agent(
                template="assistant",
                name=f"sdk-test-{timestamp}",
            )
            agent_id = agent["agent_id"]
            created_agent_id = agent_id
            print(f"Created agent: {agent_id}")

        print("\n--- Sending message ---")
        reply = client.agents.send_message(
            agent_id,
            message="Say hello in 5 words.",
        )
        print(f"Reply: {reply}")
    except LibreFangError as error:
        print(f"LibreFang request failed: {error}", file=sys.stderr)
        failed = True
    finally:
        if created_agent_id is not None:
            try:
                client.agents.kill_agent(created_agent_id, confirm=True)
                print("Agent deleted.")
            except LibreFangError as error:
                print(f"Agent cleanup failed: {error}", file=sys.stderr)
                failed = True

    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
