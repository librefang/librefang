#!/usr/bin/env python3
"""MemPalace pruner for LibreFang.

Deletes drawers older than MEMPALACE_MAX_AGE_DAYS from the palace.
Intended to be run periodically (e.g. via a LibreFang scheduled hook or cron).

NOTE: This script reads ChromaDB directly using the collection name
("mempalace_drawers") and metadata field ("filed_at") that MemPalace uses
internally. If a future MemPalace release changes these, update accordingly.

Usage:
    python3 prune.py [--dry-run]

Input (stdin):  {"type": "prune", "agent_id": "..."}   (optional, for hook mode)
Output (stdout): {"status": "...", "deleted": N, "kept": N}

Environment:
    MEMPALACE_PALACE_PATH   Path to the palace directory (default: ~/.mempalace/palace)
    MEMPALACE_MAX_AGE_DAYS  Delete drawers older than this many days (default: 90, 0 = disabled)
"""
import json
import os
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

PALACE_PATH = os.environ.get(
    "MEMPALACE_PALACE_PATH",
    os.path.expanduser("~/.mempalace/palace"),
)
MAX_AGE_DAYS = int(os.environ.get("MEMPALACE_MAX_AGE_DAYS", "90"))

# MemPalace internal constants — update if upstream changes them.
_COLLECTION_NAME = "mempalace_drawers"
_FILED_AT_FIELD = "filed_at"


def emit(obj: dict) -> None:
    json.dump(obj, sys.stdout)
    sys.stdout.write("\n")


def _parse_filed_at(value: str):  # -> datetime | None
    """Parse MemPalace's ISO timestamp into a timezone-aware datetime."""
    if not value:
        return None
    try:
        dt = datetime.fromisoformat(value)
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        return dt
    except ValueError:
        return None


def prune(dry_run: bool = False) -> dict:
    if MAX_AGE_DAYS <= 0:
        return {"status": "skip", "reason": "MEMPALACE_MAX_AGE_DAYS=0 (disabled)"}

    cutoff = datetime.now(tz=timezone.utc) - timedelta(days=MAX_AGE_DAYS)

    try:
        import chromadb
    except ImportError:
        return {"status": "error", "error": "chromadb not installed (install mempalace first)"}

    palace = Path(PALACE_PATH)
    if not palace.exists():
        return {"status": "error", "error": f"Palace path not found: {PALACE_PATH}"}

    try:
        client = chromadb.PersistentClient(path=str(palace))
        collection = client.get_collection(_COLLECTION_NAME)
    except Exception as e:
        return {"status": "error", "error": f"Could not open collection '{_COLLECTION_NAME}': {e}"}

    # Fetch all drawers. For large palaces this is O(n) in memory — acceptable
    # for a personal-scale deployment (tens of thousands of items at most).
    try:
        result = collection.get(include=["metadatas"])
    except Exception as e:
        return {"status": "error", "error": f"collection.get() failed: {e}"}

    ids = result.get("ids", [])
    metadatas = result.get("metadatas", [])

    to_delete = []
    kept = 0

    for doc_id, meta in zip(ids, metadatas):
        filed_at = _parse_filed_at((meta or {}).get(_FILED_AT_FIELD, ""))
        if filed_at is not None and filed_at < cutoff:
            to_delete.append(doc_id)
        else:
            kept += 1

    if to_delete and not dry_run:
        try:
            collection.delete(ids=to_delete)
        except Exception as e:
            return {"status": "error", "error": f"delete failed: {e}", "attempted": len(to_delete)}

    return {
        "status": "dry-run" if dry_run else "ok",
        "deleted": len(to_delete),
        "kept": kept,
        "cutoff": cutoff.isoformat(),
        "max_age_days": MAX_AGE_DAYS,
    }


def main() -> None:
    dry_run = "--dry-run" in sys.argv

    # Also accept hook-style JSON input (stdin), ignoring the payload content.
    if not sys.stdin.isatty():
        try:
            json.load(sys.stdin)
        except (json.JSONDecodeError, EOFError):
            pass

    emit(prune(dry_run=dry_run))


if __name__ == "__main__":
    main()
