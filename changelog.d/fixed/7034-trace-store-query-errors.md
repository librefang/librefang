`TraceStore::query`, `query_by_trace_id`, and `count` swallowed SQLite failures and poisoned-mutex errors, returning an empty list, `None`, or `0` indistinguishably from a genuine empty result.
A corrupt row or a failing query on the hook-trace store was therefore reported to callers as "no traces found" rather than as a failure.
These methods now return `rusqlite::Result`, and `GET /api/context-engine/traces/:trace_id` surfaces a scrubbed HTTP 500 instead of a false 404 when the store itself fails (#7034) (@houko)
