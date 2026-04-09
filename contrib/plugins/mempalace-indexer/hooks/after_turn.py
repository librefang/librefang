#!/usr/bin/env python3
"""MemPalace after_turn hook for LibreFang.

Filters conversation turns for relevant memories and saves them to MemPalace.
Skips: tool calls, short exchanges, noise, and turns where the agent already
used mcp_mempalace tools explicitly (deduplication).

Input (stdin):  {"type": "after_turn", "agent_id": "...", "messages": [...]}
Output (stdout): {"status": "..."}  (fire-and-forget)

Install: librefang plugin install mempalace-indexer && librefang plugin requirements mempalace-indexer
"""
import hashlib
import json
import os
import re
import sys
from datetime import datetime
from pathlib import Path

PALACE_PATH = os.environ.get(
    "MEMPALACE_PALACE_PATH",
    os.path.expanduser("~/.mempalace/palace"),
)
# Minimum character length of extracted text to be worth saving.
MIN_CONTENT_LENGTH = int(os.environ.get("MEMPALACE_MIN_CHARS", "80"))
# How many recent messages to consider (sliding window).
WINDOW_SIZE = int(os.environ.get("MEMPALACE_WINDOW_SIZE", "6"))
# Max content hashes to keep in the dedup store (rolling, oldest dropped first).
DEDUP_MAX = int(os.environ.get("MEMPALACE_DEDUP_MAX", "500"))

# Room classification: all matching rules win (multi-room).
# Falls back to ("default", "sessions") when nothing matches.
ROOM_RULES: list[tuple[re.Pattern, tuple[str, str]]] = [
    (
        re.compile(
            r"\b(contact|phone|email|address|family|wife|husband"
            r"|son|daughter|parent|colleague|coworker)\b"
            r"|\S+@\S+\.\w+",  # bare email address pattern
            re.IGNORECASE,
        ),
        ("people", "contacts"),
    ),
    (
        re.compile(
            r"\b(appointment|deadline|birthday|event|meeting|schedule"
            r"|remind me|reminder|calendar|due date|due on)\b",
            re.IGNORECASE,
        ),
        ("time", "calendar"),
    ),
    (
        re.compile(
            r"\b(budget|expense|transaction|payment|bill|salary|invoice"
            r"|cost|price|paid|spending|refund)\b",
            re.IGNORECASE,
        ),
        ("finance", "transactions"),
    ),
    (
        re.compile(
            r"\b(package|order|shipment|delivery|tracking|shipped|arrived)\b",
            re.IGNORECASE,
        ),
        ("logistics", "orders"),
    ),
    (
        re.compile(
            r"\b(decision|decided|prefer|from now on|going forward"
            r"|we.ll use|i.ll use|switching to|chosen|agreed)\b",
            re.IGNORECASE,
        ),
        ("knowledge", "decisions"),
    ),
]

RELEVANCE_RE = re.compile(
    r"\b(decision|decided|prefer|from now on|going forward|remember that|note that"
    r"|remind me|don.t forget|important|urgent|critical|keep in mind"
    r"|appointment|deadline|birthday|event|meeting|schedule|due date"
    r"|budget|expense|transaction|payment|bill|salary|invoice|cost|price"
    r"|package|order|shipment|delivery|tracking"
    r"|contact|phone|email|address"
    r"|like|dislike|preference|habit|allergy"
    r"|family|wife|husband|son|daughter|parent|colleague"
    r"|work|client|project|we.ll use|i.ll use|switching to)\b",
    re.IGNORECASE,
)

# Matches fenced code blocks — stripped from content before relevance checks.
CODE_BLOCK_RE = re.compile(r"```.*?```", re.DOTALL)

# Residual noise patterns after code block stripping.
NOISE_RE = re.compile(
    r"\[tool_call\]|\[tool_result\]|traceback|exception|\"type\":\s*\"tool",
    re.IGNORECASE,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def emit(obj: dict) -> None:
    json.dump(obj, sys.stdout)
    sys.stdout.write("\n")


def _classify_rooms(text: str) -> list[tuple[str, str]]:
    """Return all matching (wing, room) destinations. Falls back to default."""
    matches = [dest for pattern, dest in ROOM_RULES if pattern.search(text)]
    return matches if matches else [("default", "sessions")]


def _strip_code_blocks(text: str) -> str:
    """Replace fenced code blocks with a placeholder, preserving surrounding context."""
    return CODE_BLOCK_RE.sub("[code]", text).strip()


def _content_hash(text: str) -> str:
    """SHA-256 of the first 500 chars — stable fingerprint for near-duplicate detection."""
    return hashlib.sha256(text[:500].encode()).hexdigest()


def _dedup_path() -> Path:
    return Path(PALACE_PATH) / ".after_turn_seen.json"


def _load_seen() -> list[str]:
    path = _dedup_path()
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return []


def _save_seen(hashes: list[str]) -> None:
    path = _dedup_path()
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(hashes[-DEDUP_MAX:]))
    except OSError:
        pass  # dedup is best-effort; don't block indexing


def _is_duplicate(text: str) -> bool:
    h = _content_hash(text)
    seen = _load_seen()
    if h in seen:
        return True
    seen.append(h)
    _save_seen(seen)
    return False


def extract_text(messages):  # list[dict] -> tuple[str, bool]
    """Extract user+assistant text; detect if agent already saved to mempalace."""
    recent = messages[-WINDOW_SIZE:]
    parts = []
    agent_used_mempalace = False

    for msg in recent:
        role = msg.get("role", "")
        content = msg.get("content") or ""

        if role in ("tool", "assistant") and "mcp_mempalace" in str(content):
            agent_used_mempalace = True

        if role not in ("user", "assistant"):
            continue

        if isinstance(content, list):
            content = "\n".join(
                b.get("text", "") for b in content
                if isinstance(b, dict) and b.get("type") == "text"
            )

        if not content:
            continue

        content = _strip_code_blocks(content)

        if not content or NOISE_RE.search(content):
            continue

        parts.append(f"[{role}] {content}")

    return "\n".join(parts), agent_used_mempalace


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def main() -> None:
    try:
        data = json.load(sys.stdin)
    except (json.JSONDecodeError, EOFError):
        emit({"status": "skip", "reason": "bad input"})
        return

    messages = data.get("messages", [])
    agent_id = data.get("agent_id", "unknown")

    if not messages:
        emit({"status": "skip", "reason": "no messages"})
        return

    text, already_saved = extract_text(messages)

    if already_saved:
        emit({"status": "skip", "reason": "agent used mcp_mempalace"})
        return

    if len(text) < MIN_CONTENT_LENGTH:
        emit({"status": "skip", "reason": "too short"})
        return

    if not RELEVANCE_RE.search(text):
        emit({"status": "skip", "reason": "not relevant"})
        return

    if _is_duplicate(text):
        emit({"status": "skip", "reason": "duplicate"})
        return

    try:
        from mempalace.miner import get_collection, add_drawer

        collection = get_collection(PALACE_PATH)
        source = f"auto-{agent_id}-{datetime.now().strftime('%Y%m%d-%H%M%S%f')}"
        rooms = _classify_rooms(text)

        for wing, room in rooms:
            add_drawer(
                collection=collection,
                wing=wing,
                room=room,
                content=text,
                source_file=source,
                chunk_index=0,
                agent="mempalace-indexer",
            )

        emit({
            "status": "indexed",
            "chars": len(text),
            "rooms": [{"wing": w, "room": r} for w, r in rooms],
        })
    except Exception as e:
        emit({"status": "error", "error": str(e)})


if __name__ == "__main__":
    main()
