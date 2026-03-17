"""
LibreFang Python SDK and Client.

Two packages:
- librefang.client: REST API client for controlling LibreFang remotely
- librefang.sdk: Helper library for writing Python agents that run inside LibreFang
"""

from librefang.librefang_client import LibreFang as Client
from librefang.librefang_sdk import Agent, read_input, respond, log

__version__ = "0.5.2"

__all__ = ["Client", "Agent", "read_input", "respond", "log"]
