#!/usr/bin/env python3
"""MemPalace ingest hook for LibreFang.

Searches MemPalace for memories relevant to the incoming user message
and injects them into the agent's context as MemoryFragments.

Input (stdin):  {"type": "ingest", "agent_id": "...", "message": "user text"}
Output (stdout): {"memories": [{"content": "..."}]}

Install: librefang plugin install mempalace-indexer && librefang plugin requirements mempalace-indexer
"""
import sys
import json
import os

PALACE_PATH = os.environ.get(
    "MEMPALACE_PALACE_PATH",
    os.path.expanduser("~/.mempalace/palace"),
)
MAX_MEMORY_CHARS = int(os.environ.get("MEMPALACE_MAX_CHARS", "300"))


def emit(obj):
    """Write JSON response to stdout with trailing newline."""
    json.dump(obj, sys.stdout)
    sys.stdout.write("\n")


def truncate_at_word(text, max_len):
    """Truncate text at nearest word boundary."""
    if len(text) <= max_len:
        return text
    truncated = text[:max_len]
    last_space = truncated.rfind(" ")
    if last_space > max_len // 2:
        return truncated[:last_space] + "..."
    return truncated + "..."


def main():
    try:
        data = json.load(sys.stdin)
    except (json.JSONDecodeError, EOFError):
        emit({"memories": []})
        return

    message = data.get("message", "")
    if not message or len(message) < 5:
        emit({"memories": []})
        return

    try:
        from mempalace.searcher import search_memories

        results = search_memories(message, PALACE_PATH, n_results=5)

        memories = []
        for r in results.get("results", []):
            text = r.get("text", "")
            source = r.get("source_file", "")
            wing = r.get("wing", "")
            if text:
                snippet = truncate_at_word(text, MAX_MEMORY_CHARS)
                memories.append({"content": f"[{wing}/{source}] {snippet}"})

        emit({"memories": memories})
    except Exception as e:
        emit({"memories": [], "error": str(e)})


if __name__ == "__main__":
    main()
