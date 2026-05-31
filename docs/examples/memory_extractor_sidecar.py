#!/usr/bin/env python3
"""Reference LibreFang memory-extractor sidecar (dependency-free).

Speaks the newline-delimited JSON request/reply protocol over stdin/stdout.
The daemon sends, one object per line:

    {"id": N, "method": "extract_memories",
     "params": {"messages": [...], "categories": ["preference", ...]}}

and expects back:

    {"id": N, "ok": {"memories": [{"content": "...", "category": "preference"}, ...],
                      "has_content": true}}
    {"id": N, "error": "<msg>"}

The daemon assigns each memory a UUID, created_at, and source — you only return
the simple shape `{content, category?, level?, metadata?}`. Restrict extracted
memories to the supplied `categories`. Replace `extract` with your own logic
(your own LLM key, a local model, embeddings, whatever).

Wire it up:

    [proactive_memory.extractor_sidecar]
    command = "python3"
    args = ["/path/to/memory_extractor_sidecar.py"]

Two stdio pitfalls this avoids: read with readline() (not `for line in
sys.stdin`, which read-ahead-buffers), and flush after every reply (a long-lived
process's piped stdout is block-buffered).
"""

import json
import sys

# A toy heuristic: remember an explicit "I prefer …" / "remember that …" from
# the most recent user message. Real extractors call a model here.
TRIGGERS = ("i prefer ", "remember that ", "my name is ", "i like ", "i work ")


def extract(messages, categories):
    category = categories[0] if categories else "preference"
    memories = []
    for msg in reversed(messages):
        if msg.get("role") != "user":
            continue
        text = msg.get("content")
        if not isinstance(text, str):
            break
        low = text.strip().lower()
        for t in TRIGGERS:
            if t in low:
                memories.append({"content": text.strip(), "category": category})
                break
        break  # only inspect the most recent user message
    return {"memories": memories, "has_content": bool(memories)}


def main():
    while True:
        line = sys.stdin.readline()
        if not line:
            break
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except ValueError:
            continue
        rid = req.get("id")
        params = req.get("params", {})
        if req.get("method") == "extract_memories":
            try:
                ok = extract(params.get("messages", []), params.get("categories", []))
                reply = {"id": rid, "ok": ok}
            except Exception as exc:  # never crash the loop on one bad request
                reply = {"id": rid, "error": str(exc)}
        else:
            reply = {"id": rid, "error": f"unknown method: {req.get('method')}"}
        sys.stdout.write(json.dumps(reply) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
