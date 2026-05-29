# librefang-subprocess

Persistent JSON-over-stdio subprocess transport, shared by LibreFang's sidecar bridges.

It owns the parts every bridge was re-implementing: spawning a long-lived child, reading replies on a background task and matching them to waiters by id, bounding both the write and the reply-line size, draining stderr to the log, and reaping the child on drop.

The caller supplies a JSON request object; the transport injects an `id`, writes `{"id": N, …}`, and resolves the call with the matching reply (`{"id": N, "ok": …}` or `{"id": N, "error": …}`).

Lives below `librefang-channels` and `librefang-runtime` in the dependency graph and depends on no `librefang-*` crate.

See `docs/architecture/sidecar-context-engine.md` for the first consumer and `crates/librefang-subprocess/src/lib.rs` for the API.
