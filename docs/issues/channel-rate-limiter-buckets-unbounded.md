# [Medium] Channel bridge hardening — rate-limiter buckets, cooperative abort, sanitizer bypass, biased select

**Severity:** Medium
**Category:** Channels · DoS · Shutdown
**Status:** Merges 3 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Severity | Description |
|--------|----------|-------------|
| this | Medium | `ChannelRateLimiter.buckets: DashMap<String, ...>` only retains timestamps; **entries are never evicted**; a flood of synthetic `platform_id`s exhausts memory |
| cooperative abort | Low | Channel-adapter `abort()` is only cooperative; the inner spawned loop does not exit immediately |
| sanitizer bypass | Low | The separator regex in `sanitizer.rs` can be circumvented by rewriting (e.g. `"S y s t e m :"`, single-dash separators) |
| biased select | Low | The inbound `select!` lacks `biased;`, so during shutdown a new `dispatch_message` may be spawned on the same poll as `shutdown.changed()` |

## Affected files

- `crates/librefang-channels/src/rate_limiter.rs:22-58` (`buckets` map)
- `crates/librefang-channels/src/bridge.rs:1302-1348` (inbound `select!`)
- `crates/librefang-channels/src/bridge.rs:1463-1505` (`approval_listener`, already uses `biased;` — reference)
- `crates/librefang-channels/src/sanitizer.rs:47-114, 120-151`
- `crates/librefang-channels/src/bridge.rs` (cooperative `abort` loops)

## Why merged

All four are channel-bridge hygiene items adjacent to the main rate-limit bypass tracked in [channel-bridge-bypasses-lane-semaphore](channel-bridge-bypasses-lane-semaphore.md). Any fix touches `bridge.rs` + `rate_limiter.rs` + `sanitizer.rs` together.

## Combined fix plan

1. **Rate-limiter entry eviction (this)**: a periodic sweep (30s) calls `retain` and then drops empty `SmallVec` entries; cap `buckets.len()` and LRU-evict on overflow. Consider stacking a per-source-IP token bucket at the `/channels` nest layer (webhook signature guarantees authenticity, not rate).
2. **Cooperative abort → explicit (cooperative abort)**: when spawning subtasks, attach a `CancellationToken`; `abort()` triggers cancel, and the subloop responds at the top-level `select!`. Or switch to a clearer API like `JoinSet::shutdown()`.
3. **Document sanitizer boundary (sanitizer bypass)**: in documentation and inline comments, make explicit that "`SanitizeResult::Clean` is not a security boundary." Keep it as a cheap first line; downstream prompt assembly must still fence channel content inside a tagged block, with `injection_guard.rs:85` as the second line.
4. **Standardize `biased;` (biased select)**: every channel-adapter `select!` adds `biased;` with `shutdown.changed()` as the first arm. Align with the `approval_listener:1466-1472` comment template.

## Tests

- (this) Synthesize 10000 distinct `platform_id`s; `buckets` size stays ≤ `MAX_BUCKETS`; after 30s, empty entries are evicted.
- (cooperative abort) After `abort()`, the inner loop exits within ≤ 100 ms.
- (sanitizer bypass) Sanitizer returns `Clean` for `"S y s t e m :"`, but the downstream fence is still correct.
- (biased select) When the shutdown signal fires, no new `dispatch_message` is spawned.
