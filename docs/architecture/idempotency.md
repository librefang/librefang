# Idempotency-Key (#3637)

State-creating `POST` endpoints accept an optional `Idempotency-Key`
header so a duplicate request — same key, same body — replays the
prior response instead of executing the handler twice. This closes the
class of bugs where a dashboard double-click, a network retry, or a
channel webhook redelivery silently created two of something.

## Behaviour

| Condition | Outcome |
|---|---|
| No `Idempotency-Key` header | Handler runs as before. No state recorded. |
| First request with key `K` (2xx response) | Response cached for 24h under `(K, sha256(body))`. |
| First request with key `K` (4xx / 5xx) | Not cached. Slot stays free; clients can retry. |
| Concurrent request with key `K` + same body | `409 Conflict` with `code = "idempotency_key_in_use"`; the second handler does not run. |
| Repeat with key `K` + same body | Replays cached `(status, body)` byte-for-byte. Inner handler does **not** run. |
| Repeat with key `K` + different body | `409 Conflict` with `code = "idempotency_key_conflict"`. |
| Empty / oversize / non-printable key | `400 Bad Request` with `code = "idempotency_key_invalid"`. |

The 24-hour replay window starts when a successful handler completes; it is long enough to absorb realistic dashboard / webhook redelivery races without retaining completed responses indefinitely.
Expired completed rows are deleted lazily on lookup and by opportunistic pruning, which is limited to once per minute per shared store.

Body identity is sha256 over the raw JSON bytes the handler received
(not the parsed value) so a re-serialised body with reordered keys
mismatches. Callers that need canonicalisation should do it before
sending.

## Endpoints supported (this PR)

- `POST /api/agents` — spawning is the highest-cost duplicate to
  recover from (creates a kernel-tracked agent, allocates a workspace,
  optionally pulls model config).
- `POST /api/a2a/send` — outbound A2A task dispatch is the
  network-flakiest path; client retries are a routine occurrence and
  every duplicate spends real upstream tokens.

Both routes are unchanged for callers that omit the header.

## Out of scope (follow-up under #3637)

The remaining state-creating POSTs called out in the issue land in
follow-up PRs in this series:

- `POST /api/hands/{name}/activate` — hand instance lifecycle
- `POST /api/plugins/install` — plugin install
- `POST /api/webhooks` — channel webhook subscription
- Per-channel inbound dedup (e.g. Telegram `update_id` reuse) — a
  separate concern routed through `librefang-channels`, not this
  middleware

## Persistence

Schema is migration v34 in `librefang-memory`:

```sql
CREATE TABLE idempotency_keys (
    key             TEXT PRIMARY KEY,
    body_hash       TEXT NOT NULL,
    response_status INTEGER NOT NULL,
    response_body   BLOB NOT NULL,
    created_at      INTEGER NOT NULL,
    expires_at      INTEGER NOT NULL
);
CREATE INDEX idx_idempotency_keys_expires_at
    ON idempotency_keys(expires_at);
```

The store reuses the `MemorySubstrate` SQLite pool so every byte sits under one WAL pool — no separate database file and no second database.
Reservation uses `BEGIN IMMEDIATE` plus `INSERT OR IGNORE` with `response_status = 0` as the pending sentinel and a random owner token in `response_body`.
The transaction has exactly one owner before any handler starts, so concurrent same-key requests cannot both perform side effects.
Only that token's owner may complete or release the row, preventing a superseded request from modifying a replacement reservation.
Successful handlers replace their pending row with the replayable status and body; non-success and cancelled handlers release it.

Pending reservations do not use the replay TTL and are not reclaimed automatically.
This keeps a handler that legitimately runs for more than 24 hours from losing its reservation to a retry.
If the process exits abnormally before the guard can release the row, the key remains fail-closed until an operator verifies the original side effect and explicitly removes that orphaned pending row.

## Failure modes

- **Reservation error before execution**: the middleware returns `503 idempotency_store_unavailable` and does not run the handler because it cannot promise deduplication.
- **Completion error after a successful handler**: the middleware returns the same 503 and leaves the pending reservation in place, failing closed instead of making a completed side effect immediately retriable.
- **Invalid system time or persisted status**: the store rejects the operation instead of inventing an expiry timestamp or truncating the corrupt status.
- **Concurrent in-flight duplicate with the same body**: returns `409 idempotency_key_in_use` without starting the second handler, matching Stripe's documented conflict behavior.
- **Cancellation or panic**: the reservation guard releases its token-owned pending row, so a later retry can make a new attempt.
