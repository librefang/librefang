`GET /api/agents/{id}/files` now probes workspace identity-file existence and size on a `spawn_blocking` task instead of calling `std::fs::metadata` inline on the async handler.
  The per-file `.identity/` vs workspace-root fallback check previously ran as two separate `exists()` stats followed by a third `metadata()` call directly on a Tokio worker thread, parking it on disk I/O for every probed file on every request.
  The listing now runs as a single batched blocking task, and each probe collapses to one `metadata()` call instead of a redundant `exists()` + `metadata()` pair.
  A failed blocking task returns a scrubbed 500 rather than propagating the raw `JoinError` (#6980) (@houko)
