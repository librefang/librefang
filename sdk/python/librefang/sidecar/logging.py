"""Structured stderr logging for sidecar adapters.

stdout is the JSON-RPC protocol channel — it MUST carry only protocol
frames. Every diagnostic line goes to stderr, which LibreFang forwards
into the daemon log. Use these helpers (or your own stderr writer);
never ``print()`` to stdout from an adapter.
"""

from __future__ import annotations

import json
import os
import sys
import time
from typing import Any


_LEVEL_VALUES = {
    "debug": 10,
    "info": 20,
    "warn": 30,
    "warning": 30,
    "error": 40,
}
_LOG_FAILURE = b"[sidecar log failure] could not write structured log\n"


def _enabled(level: str) -> bool:
    configured = os.environ.get("LIBREFANG_LOG_LEVEL", "debug").strip().lower()
    minimum = _LEVEL_VALUES.get(configured, _LEVEL_VALUES["debug"])
    return _LEVEL_VALUES.get(level.lower(), minimum) >= minimum


def log(level: str, message: str, **fields: Any) -> None:
    """Write one structured JSON log line to stderr."""
    if not _enabled(level):
        return
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
        try:
            os.write(2, _LOG_FAILURE)
        except Exception:
            pass


def debug(message: str, **fields: Any) -> None:
    log("debug", message, **fields)


def info(message: str, **fields: Any) -> None:
    log("info", message, **fields)


def warn(message: str, **fields: Any) -> None:
    log("warn", message, **fields)


def error(message: str, **fields: Any) -> None:
    log("error", message, **fields)
