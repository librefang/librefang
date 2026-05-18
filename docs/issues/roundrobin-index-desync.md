# [Medium] `credential_pool.rs` hygiene — index desync, `mark_success` race, biased rotation, weak randomness, duplicate-key collision

**Severity:** Medium
**Category:** Credentials · Concurrency
**Status:** Merges 4 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Severity | Description |
|--------|----------|-------------|
| this | Medium | RoundRobin index goes out of bounds / points at the wrong key after `mark_exhausted` or hot-reload |
| mark_success clears permanent | Medium | `mark_success` unconditionally clears the `exhausted_until` set by `mark_permanent` → permanently failed keys are resurrected |
| biased rotation | Medium | `acquire_round_robin` uses `find(\|&&i\| i >= start_idx % n)` linear scan + double-recomputes `available`; the cursor advances with a bias toward low-priority slots |
| weak randomness | Low | `acquire_random` seeds from `SystemTime::subsec_nanos()` → under bursty traffic the selection sequence is predictable |
| duplicate-key drift | Low | `mark_exhausted` / `mark_success` both use `find()` to mutate only the first matching `api_key`, so duplicates silently drift apart |

## Affected files

- `crates/librefang-llm-drivers/src/credential_pool.rs:207-240` (RoundRobin body)
- `crates/librefang-llm-drivers/src/credential_pool.rs:245-281` (the three `mark_*` functions)
- `crates/librefang-llm-drivers/src/credential_pool.rs:213-238` (index maintenance)
- `crates/librefang-llm-drivers/src/credential_pool.rs:327-344` (`acquire_round_robin`)
- `crates/librefang-llm-drivers/src/credential_pool.rs:346-360` (`acquire_random`)

## Why merged

All five are hygiene problems with the `CredentialPool` state machine; any fix touches the `mark_*` / `acquire_*` function groups.

## Combined fix plan

1. **Dedup at construction (duplicate-key drift)**: `CredentialPool::new` dedupes `keys` by `api_key`, warning on duplicates:
   ```rust
   keys.retain(|k| seen.insert(k.api_key.clone()) || { warn!(...); false });
   ```
2. **Make `mark_*` semantics precise (mark_success clears permanent / duplicate-key drift)**: add a `permanent: bool` field to `PooledCredential`; `mark_success` early-returns without clearing state when `permanent == true`.
3. **Round-robin refactor (this / biased rotation)**: `acquire_round_robin` returns `(api_key, next_idx)`; the outer cursor advance and the `available` view come from the same computation. Use `iter().cycle().skip(start).take(n)` semantics; do not compare absolute indices via `>=`. After `mark_exhausted` / hot-reload, fix up once with `self.index %= self.active_keys.len().max(1)`.
4. **Cryptographically strong random for `acquire_random` (weak randomness)**:
   ```rust
   let idx = rand::thread_rng().gen_range(0..available.len());
   ```
   Or call `getrandom::getrandom` directly.

## Tests

- (this / biased rotation) Three-key pool: `mark_exhausted` on the middle key, the next acquire returns one of the remaining two — no panic, no skip.
- (mark_success clears permanent) `mark_permanent` followed immediately by `mark_success` → `is_available()` stays false.
- (weak randomness) 1000 calls to `acquire_random`: χ² test shows an approximately uniform distribution.
- (duplicate-key drift) Duplicate input keys → construction emits `warn!`; runtime `mark_*` applies to all duplicates.
- Integration: a first-attempt 429 **rotates** to the next key (joint test with [pooled-driver-no-invalidate](pooled-driver-no-invalidate.md)).
