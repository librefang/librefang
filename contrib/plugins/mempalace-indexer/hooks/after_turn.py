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

RELEVANCE_RE = re.compile(
    r"\b(decision|decided|prefer|from now on|remember that"
    r"|appointment|deadline|birthday|event|meeting"
    r"|budget|expense|transaction|payment|bill|salary"
    r"|package|order|shipment|delivery|tracking"
    r"|contact|phone|email|address"
    r"|important|urgent|critical|don.t forget"
    r"|like|dislike|preference|habit|allergy"
    r"|family|wife|husband|son|daughter|parent"
    r"|work|client|project|invoice)\b",
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

        add_drawer(
            collection=collection,
            wing="default",
            room="sessions",
            content=text,
            source_file=source,
            chunk_index=0,
            agent="mempalace-indexer",
        )

        emit({"status": "indexed", "chars": len(text)})
    except Exception as e:
        emit({"status": "error", "error": str(e)})


if __name__ == "__main__":
    main()
