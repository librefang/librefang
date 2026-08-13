`/api/dashboard/snapshot` now runs its database health probe and session-count query together on Tokio's blocking pool instead of inline on the async worker.
  Both calls go through the synchronous SQLite substrate, so a slow disk could previously stall the worker thread handling the dashboard's 5 s poll.
  A blocking-task failure now logs at `error` level and falls back to the existing degraded-health / zero-count semantics instead of silently collapsing. (#6989) (@houko)
