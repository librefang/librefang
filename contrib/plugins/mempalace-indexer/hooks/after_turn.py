#!/usr/bin/env python3
"""MemPalace after_turn hook for LibreFang.

Filters conversation turns for relevant memories and saves them to MemPalace.
Skips: tool calls, short exchanges, noise, and turns where the agent already
used mcp_mempalace tools explicitly (deduplication).

Input (stdin):  {"type": "after_turn", "agent_id": "...", "messages": [...]}
Output (stdout): {"status": "..."}  (fire-and-forget)

Install: librefang plugin install mempalace-indexer && librefang plugin requirements mempalace-indexer
"""
import sys
import json
import re
import os
from datetime import datetime

PALACE_PATH = os.environ.get(
    "MEMPALACE_PALACE_PATH",
    os.path.expanduser("~/.mempalace/palace"),
)
MIN_CONTENT_LENGTH = 80

# Room classification: keywords → (wing, room).
# Evaluated in order; first match wins. Falls back to ("default", "sessions").
ROOM_RULES: list[tuple[re.Pattern, tuple[str, str]]] = [
    (
        re.compile(
            r"\b(contact|phone|email|address|family|wife|husband"
            r"|son|daughter|parent|colleague|coworker)\b",
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

NOISE_RE = re.compile(
    r"\[tool_call\]|\[tool_result\]|```|traceback|exception|\"type\":\s*\"tool",
    re.IGNORECASE,
)


def emit(obj):
    """Write JSON response to stdout with trailing newline."""
    json.dump(obj, sys.stdout)
    sys.stdout.write("\n")


def _classify_room(text: str) -> tuple[str, str]:
    """Return (wing, room) for the given text based on ROOM_RULES."""
    for pattern, destination in ROOM_RULES:
        if pattern.search(text):
            return destination
    return ("default", "sessions")


def extract_text(messages):
    """Extract user+assistant text, detect if agent already saved to mempalace."""
    recent = messages[-6:]
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

        if not content or NOISE_RE.search(content):
            continue

        parts.append(f"[{role}] {content}")

    return "\n".join(parts), agent_used_mempalace


def main():
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

    try:
        from mempalace.miner import get_collection, add_drawer

        collection = get_collection(PALACE_PATH)
        source = f"auto-{agent_id}-{datetime.now().strftime('%Y%m%d-%H%M%S%f')}"

        wing, room = _classify_room(text)

        add_drawer(
            collection=collection,
            wing=wing,
            room=room,
            content=text,
            source_file=source,
            chunk_index=0,
            agent="mempalace-indexer",
        )

        emit({"status": "indexed", "chars": len(text), "wing": wing, "room": room})
    except Exception as e:
        emit({"status": "error", "error": str(e)})


if __name__ == "__main__":
    main()
