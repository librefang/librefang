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
| Repeat with key `K` + same body | Replays cached `(status, body)` byte-for-byte. Inner handler does **not** run. |
| Repeat with key `K` + different body | `409 Conflict` with `code = "idempotency_key_conflict"`. |
| Empty / oversize / non-printable key | `400 Bad Request` with `code = "idempotency_key_invalid"`. |

The 24-hour window is the recommended default from the issue; long
enough to absorb realistic dashboard / webhook redelivery races, short
enough that a key the operator forgets about doesn't pin replayable
state forever. `expires_at = created_at + 86400`. Expired rows are
deleted lazily on the next lookup that hits the same key, plus
opportunistically after every cache miss.

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

The store reuses `MemorySubstrate::usage_conn()` so every byte sits
under the same WAL pool — no separate database file, no second open
call. First-writer-wins is enforced via `INSERT OR IGNORE`, so a race
between two duplicate requests resolves deterministically: whichever
writer's `INSERT` lands first owns the canonical reply.

## Failure modes

- **Lookup error**: the middleware logs and falls through to "execute
  the handler anyway, don't cache". A corrupt cache row can never
  block real traffic.
- **Persist error after the handler succeeded**: logged at `warn`, the
  successful response is still returned. The next duplicate request
  re-executes the handler — the same property as a non-2xx first
  response.
- **Concurrent in-flight duplicates**: both requests run the handler;
  whichever finishes first writes the cache row, the loser's `INSERT
  OR IGNORE` is a no-op. Both clients see a 2xx, but they may not be
  byte-identical (e.g. two `agent_id`s). This is the same property as
  Stripe's idempotency-key implementation; for stronger semantics
  (single-flight) callers should serialize at the dispatcher.
