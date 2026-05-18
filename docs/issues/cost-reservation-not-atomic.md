# [Medium] Concurrency hygiene roundup — non-atomic `CostReservation` + session DEFERRED tx hitting `SQLITE_BUSY`

**Severity:** Medium · **Domain:** Concurrency
**Status:** Merges 1 earlier issue into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | `CostReservationLedger::reserve_global_budget` splits check and add into two phases (dropping the mutex in between); two concurrent callers can both pass the gate and add, exceeding `max_hourly_usd` | `librefang-kernel-metering/src/lib.rs:182-231`, `:41-51` |
| session BUSY | Session save runs inside a DEFERRED tx; under contention SQLite returns `SQLITE_BUSY_SNAPSHOT` instead of auto-retrying | `librefang-memory/src/session.rs` (save section) |

## Why merged

Both are "check-then-act" race-hygiene items in the kernel / memory layer; any PR touches lock acquisition / transaction boundaries.

## Combined fix plan

1. **(this) Single critical section**:
   ```rust
   let mut pending = self.pending.lock();
   if pending.current() + projection > max_hourly_usd { return Err(...); }
   pending.add(projection);
   ```
2. **(session BUSY) Upgrade the tx type + retry**:
   - Replace `DEFERRED` with SQLite `BEGIN IMMEDIATE` (acquires the write lock up front, avoiding BUSY at promotion);
   - On `SQLITE_BUSY_SNAPSHOT`, do a bounded retry (max 3 attempts, exponential backoff).

## Tests

- (this) `#[tokio::test(flavor = "multi_thread")]` with 20 concurrent `reserve` calls → total committed ≤ `max_hourly_usd`.
- (session BUSY) 100 concurrent session saves → zero unrecoverable BUSY; P99 latency within target.
