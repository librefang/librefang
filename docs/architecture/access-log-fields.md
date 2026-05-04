# HTTP access log structured fields

LibreFang's `request_logging` middleware (`crates/librefang-api/src/middleware.rs`)
emits one structured `tracing` event per HTTP request. Operators grep these
lines to trace requests across the kernel boundary; the kernel orchestration
spans and the per-driver `llm.complete` / `llm.stream` spans inherit the
same `request_id` field, so a single grep on `request_id=<uuid>` lights up
the full execution path.

This document is the canonical reference for the shape of those events.

## Field schema

Every access-log event carries the following structured fields:

| Field        | Source                                                 | Cardinality / notes                                                |
|--------------|--------------------------------------------------------|--------------------------------------------------------------------|
| `request_id` | `request_logging` middleware (UUID v4)                 | Per-request, unique. Inherited by every child span.                |
| `method`     | `request.method()`                                     | HTTP verb, low cardinality.                                        |
| `path`       | `request.uri().path()`                                 | Raw URI path. UUIDs are NOT normalized in the log line.            |
| `status`     | Response status code                                   | 100–599.                                                           |
| `latency_ms` | `Instant::now() - start`                               | Wall-clock latency in milliseconds.                                |
| `agent_id`   | Response extension `AgentIdField` (see below)          | Empty string when the route does not carry an agent in its path.   |

The level of the event depends on `status`:

- `5xx` → `error!` (server faults must surface).
- `4xx` → `warn!` (auth storms, validation errors).
- `2xx` / `3xx` `GET` → `debug!` (poll noise suppressed by default).
- everything else → `info!`.

## How `agent_id` gets there

The middleware itself only sees the raw URI; `path` normalization in
`metrics.rs` collapses agent UUIDs to `{id}` (Prometheus label hygiene),
so the path-substring fallback no longer works either. To attach a
structured `agent_id` field without forcing every handler to take a
`tracing::Span` argument, handlers that have already extracted a typed
`AgentId` from the request path drop a marker into `Response::extensions`:

```rust
// crates/librefang-api/src/extensions.rs
pub struct AgentIdField(pub AgentId);

pub fn with_agent_id<R: IntoResponse>(agent_id: AgentId, body: R) -> Response {
    let mut response = body.into_response();
    response.extensions_mut().insert(AgentIdField(agent_id));
    response
}
```

The middleware reads it back after `next.run().await`:

```rust
let agent_id = response
    .extensions()
    .get::<crate::extensions::AgentIdField>()
    .map(|f| f.0.to_string());
```

Handlers opt into the enrichment by ending their happy and error paths
with `crate::extensions::with_agent_id(agent_id, body)`. The call is a
no-op for the wire format — extensions are an in-process channel
between handler and middleware and never cross the wire.

Closes [#3511](https://github.com/librefang/librefang/issues/3511).

## Cardinality and metrics: do NOT add `agent_id` as a Prometheus label

`agent_id` is a UUID and effectively unbounded. It belongs in the
`tracing` event stream, where lines are written and discarded, but it
must NOT be added to any Prometheus label set in `metrics.rs` — every
new agent would mint a new time series and blow up the metric
cardinality budget. The path-normalization to `{id}` in
`crates/librefang-api/src/metrics.rs` is deliberate for this reason.

If you need per-agent metrics, aggregate them in the kernel into
bounded buckets (`ok` / `4xx` / `5xx`, or coarse latency histograms)
before exposing them.

## Coverage

As of the PR closing the bulk of [#3511](https://github.com/librefang/librefang/issues/3511),
`with_agent_id` is wired up in the hot-path handlers under:

- `routes/agents.rs` — agent CRUD, suspend/resume, kill, mode, status.
- `routes/auto_dream.rs` — trigger / abort / set-enabled.
- `routes/budget.rs` — `agent_budget_status`, `update_agent_budget`.
- `routes/memory.rs` — KV list/get/set/delete and import/export.
- `routes/prompts.rs` — list/create prompt versions and experiments.

Handlers whose `agent_id` lives in the request body (e.g.
`webhooks::webhook_agent`, `network::a2a_send_task`) or in a query
string are not yet covered; the path is taken as the source of truth
for the access-log marker and body-derived agents are tracked as a
follow-up under #3511.

## Follow-ups still tracked under #3511

- **`session_id`** — surfacing the resolved `SessionId` from
  `KernelHandle::send_message` requires extending the kernel↔API
  trait surface; that is its own review.
- **Auto-extractor layer** — making `AgentIdPath` (and any future
  `AgentScopedPath` extractors) drop the marker for free, so the
  per-handler `with_agent_id(...)` call goes away.
- **Body-derived agent IDs** — endpoints that take `agent_id` in the
  JSON body need a different injection point (after deserialization)
  and are out of scope for the path-marker pattern documented here.
