"""Structured stderr logging for sidecar adapters.

stdout is the JSON-RPC protocol channel — it MUST carry only protocol
frames. Every diagnostic line goes to stderr, which LibreFang forwards
into the daemon log. Use these helpers (or your own stderr writer);
never ``print()`` to stdout from an adapter.
"""

from __future__ import annotations

import json
import sys
import time
from typing import Any


def log(level: str, message: str, **fields: Any) -> None:
    """Write one structured JSON log line to stderr."""
    record = {
        "ts": time.time(),
        "level": level,
        "message": message,
    }
    if fields:
        record["fields"] = fields
    try:
        sys.stderr.write(json.dumps(record, default=str) + "\n")
        sys.stderr.flush()
    except Exception:
        # Logging must never take the adapter down.
        pass


def debug(message: str, **fields: Any) -> None:
    log("debug", message, **fields)


def info(message: str, **fields: Any) -> None:
    log("info", message, **fields)


def warn(message: str, **fields: Any) -> None:
    log("warn", message, **fields)


def error(message: str, **fields: Any) -> None:
    log("error", message, **fields)
