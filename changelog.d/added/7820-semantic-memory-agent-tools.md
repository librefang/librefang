**Agents can finally ask their own semantic memory a question.**
Embeddings, cosine re-ranking, confidence decay and consolidation have been in LibreFang for a long time, but the entire tool surface an agent got for "memory" was three exact-key operations against `kv_store`.
Semantic memory was reachable only automatically — the recall the agent loop injects before each turn — or externally, over the REST API.
An agent could be *given* a memory it never asked for and could never ask for one, could not see what it had stored, and could not retract a memory that had gone stale.
That last one is the expensive failure: a memory saying "service X is unavailable" keeps being recalled into context long after the outage ends, and the agent has no way to look at it, correct it, or merge the near-duplicates reinforcing it.
Four new tools close the seam — `memory_semantic_search` (with a `min_confidence` floor, so a caller can ask for nothing rather than stale noise), `memory_semantic_add`, `memory_semantic_forget`, and `memory_semantic_stats`.
The `memory_semantic_` prefix is deliberate: `memory_store` (keyed) and a hypothetical `memory_remember` (keyless) read as synonyms and get picked at random.
Only `memory_semantic_search` ships its schema on every turn; the rest stay behind `tool_search` / `tool_load`.
Agents that reached this store through an MCP bridge wrapping the daemon's own REST API — a stdio child looping back over HTTP to the process it was spawned from — no longer need one, and no longer lose all addressable long-term memory when that child dies. (#7820) (@houko)
