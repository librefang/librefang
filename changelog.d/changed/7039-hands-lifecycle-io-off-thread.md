`POST /api/hands/{id}/pause|resume|deactivate` and `POST /api/hands/reload` ran hand-registry persistence, and for activation/deactivation the workspace and SQLite I/O behind it, synchronously on the async worker thread handling the request.
  All five lifecycle operations now run their kernel call through `tokio::task::spawn_blocking` so the request handler never parks on disk I/O.
  Successful responses and existing business-error status codes are unchanged; a join failure on the blocking task now returns a scrubbed 500 instead of propagating as an unhandled panic (#7039) (@houko)
