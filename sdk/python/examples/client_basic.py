#!/usr/bin/env python3
"""
Basic example — create an agent and chat with it via the REST API.

Usage:
    python client_basic.py
"""

import sys
import os
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from librefang import Client as LibreFang

client = LibreFang("http://localhost:4545")

# Check server health
health = client.health()
print("Server:", health)

# List existing agents
agents = client.agents.list()
print(f"Agents: {len(agents)}")

# Use existing agent or create a new one with unique name
if agents:
    agent = agents[0]
    print(f"Using existing agent: {agent['id']}")
    should_delete = False
else:
    timestamp = int(time.time())
    agent = client.agents.create(template="assistant", name=f"sdk-test-{timestamp}")
    print(f"Created agent: {agent['id']}")
    should_delete = True

# Send a message and get the full response
print("\n--- Sending message ---")
reply = client.agents.message(agent["id"], "Say hello in 5 words.")
print(f"Reply: {reply}")

# Clean up only if we created it
if should_delete:
    client.agents.delete(agent["id"])
    print("Agent deleted.")
