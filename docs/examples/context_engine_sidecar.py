#!/usr/bin/env python3
"""Reference LibreFang context-engine sidecar (dependency-free).

Speaks the newline-delimited JSON request/reply protocol described in
docs/architecture/sidecar-context-engine.md. This reference recalls no
memories and trims the window to its most recent slice; replace the bodies
of `ingest` / `assemble` / `after_turn` with your own policy.

Wire it up:

    [context_engine]
    engine = "sidecar"

    [context_engine.sidecar]
    command = "python3"
    args = ["/path/to/context_engine_sidecar.py"]

Two stdio pitfalls this avoids:
  * read with readline(), not `for line in sys.stdin` (the latter
    read-ahead-buffers and would not yield a single line until EOF);
  * flush after every reply (a long-lived process's stdout is block-buffered
    when it is a pipe, so the reply would never reach the daemon otherwise).
"""

import json
import sys

# Keep at most this many of the most recent messages in the assembled window.
KEEP_RECENT = 40


def ingest(params):
    # No recall in this reference. Return memory fragments to inject them into
    # the system prompt — see MemoryFragment in librefang-types for the shape.
    return {"recalled_memories": []}


def assemble(params):
    messages = params.get("messages", [])
    kept = messages[-KEEP_RECENT:]
    removed = len(messages) - len(kept)
    recovery = "None" if removed == 0 else {"AutoCompaction": {"removed": removed}}
    return {"messages": kept, "recovery": recovery}


def after_turn(params):
    # Persist state, update indexes, etc. Nothing to do here.
    return {}


def bootstrap(params):
    return {}


HANDLERS = {
    "ingest": ingest,
    "assemble": assemble,
    "after_turn": after_turn,
    "bootstrap": bootstrap,
}


def main():
    while True:
        line = sys.stdin.readline()
        if not line:
            break  # EOF — daemon closed stdin
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except ValueError:
            continue
        rid = req.get("id")
        method = req.get("method", "")
        handler = HANDLERS.get(method)
        if handler is None:
            reply = {"id": rid, "error": f"unknown method: {method}"}
        else:
            try:
                reply = {"id": rid, "ok": handler(req.get("params", {}))}
            except Exception as exc:  # never crash the loop on one bad request
                reply = {"id": rid, "error": str(exc)}
        sys.stdout.write(json.dumps(reply) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
