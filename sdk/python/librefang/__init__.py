"""
LibreFang Python SDK and Client.

Three packages:
- librefang.client: REST API client for controlling LibreFang remotely
- librefang.sdk: Helper library for writing Python agents that run inside LibreFang
- librefang.sidecar: Framework for writing out-of-process channel adapters
"""

from librefang.librefang_client import LibreFang as Client
from librefang.librefang_sdk import Agent, read_input, respond, log

__version__ = "0.5.2"

__all__ = ["Client", "Agent", "read_input", "respond", "log"]

# `librefang.sidecar` is intentionally NOT re-exported here: a sidecar
# adapter does `from librefang.sidecar import ...` explicitly, and
# importing it eagerly would pull asyncio/threading into every
# REST-client user. Import the subpackage on demand.
