#!/usr/bin/env python3
"""Example LibreFang agent: echoes back messages with a friendly greeting."""

import os

from librefang import Agent

agent = Agent()


@agent.on_message
def handle(message: str, context: dict) -> str:
    agent_id = context.get("agent_id", os.environ.get("LIBREFANG_AGENT_ID", "unknown"))
    return f"Hello from Python agent {agent_id}! You said: {message}"


if __name__ == "__main__":
    agent.run()
