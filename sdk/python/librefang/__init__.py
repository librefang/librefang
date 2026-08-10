"""
LibreFang Python SDK and Client.

Three packages:
- librefang.client: REST API client for controlling LibreFang remotely
- librefang.sdk: Helper library for writing Python agents that run inside LibreFang
- librefang.sidecar: Framework for writing out-of-process channel adapters
"""

import re
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path

from librefang.librefang_client import LibreFang as Client
from librefang.librefang_sdk import Agent, read_input, respond, log


def _package_version() -> str:
    source_pyproject = Path(__file__).resolve().parents[1] / "pyproject.toml"
    if source_pyproject.is_file():
        match = re.search(
            r'^version\s*=\s*"([^"]+)"',
            source_pyproject.read_text(encoding="utf-8"),
            re.MULTILINE,
        )
        if match is not None:
            return match.group(1)

    try:
        return version("librefang-sdk")
    except PackageNotFoundError:
        return "0+unknown"


__version__ = _package_version()

__all__ = ["Client", "Agent", "read_input", "respond", "log"]

# `librefang.sidecar` is intentionally NOT re-exported here: a sidecar
# adapter does `from librefang.sidecar import ...` explicitly, and
# importing it eagerly would pull asyncio/threading into every
# REST-client user. Import the subpackage on demand.
