# librefang-memory

Memory substrate for the [LibreFang](https://github.com/librefang/librefang) Agent OS.

Provides a unified memory API across three storage backends:

- **Structured store** (SQLite, in `structured`) — key/value pairs,
  sessions, agent state, audit trail.
- **Semantic store** (`semantic`, `http_vector_store`) — text search;
  LIKE-based today, Qdrant-backed in the vector path.
- **Knowledge graph** (`knowledge`, SQLite-backed) — entities and
  relations.

Agents interact with a single `Memory` trait that abstracts all three.

## Proactive memory (mem0-style)

The `proactive` module exposes:

- `ProactiveMemory` — unified API: `search`, `add`, `get`, `list`.
- `ProactiveMemoryHooks` — auto-memorize / auto-retrieve hooks.
- `ProactiveMemoryStore` — implementation on top of `MemorySubstrate`.

Plus: `chunker`, `consolidation`, `decay`, `migration`,
`namespace_acl`, `prompt`, `provider`, `roster_store`, `session`.

## Key dependencies

`librefang-types`, `tokio`, `serde`, `serde_json`, `rmp-serde`,
`rusqlite` (with FTS5), `chrono`.

See the [workspace README](../../README.md) and
[architecture docs](../../docs/architecture/).
