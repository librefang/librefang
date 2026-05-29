# librefang-subprocess

Persistent JSON-over-stdio subprocess transport, shared by LibreFang's sidecar bridges.

It owns the parts every bridge was re-implementing: spawning a long-lived child, reading replies on a background task and matching them to waiters by id, bounding both the write and the reply-line size, draining stderr to the log, and reaping the child on drop.

The caller supplies a JSON request object; the transport injects an `id`, writes `{"id": N, …}`, and resolves the call with the matching reply (`{"id": N, "ok": …}` or `{"id": N, "error": …}`).

Two layers are exposed.
`SubprocessTransport` is the raw id-matched transport above — one child, dead once the child exits.
`SupervisedTransport` wraps it for callers that want resilience: it spawns the child lazily on first use and re-spawns it after a crash (rate-limited by a respawn cooldown), so a transient sidecar failure degrades a single call rather than the daemon's whole lifetime.

Lives below `librefang-channels` and `librefang-runtime` in the dependency graph and depends on no `librefang-*` crate.

The context engine (`docs/architecture/sidecar-context-engine.md`) and the proactive-memory extractor are the in-tree consumers; see `crates/librefang-subprocess/src/lib.rs` for the API.
