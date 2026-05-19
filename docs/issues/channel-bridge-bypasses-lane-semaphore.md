# Channel bridge dispatch bypasses the kernel lane semaphore; spawn rate is unbounded

**Severity:** High
**Category:** Command injection, SSRF, sandbox (concurrency isolation)
**Labels:** `security`, `concurrency`, `dos`, `high`

## Affected files
- `crates/librefang-channels/src/bridge.rs:1297` (per-adapter semaphore)
- `crates/librefang-channels/src/bridge.rs:1317-1332` (inbound `select!` / spawn)
- `crates/librefang-channels/src/bridge.rs:2827` (`dispatch_message`)

## Description

Each channel adapter carries its own `tokio::sync::Semaphore::new(32)` and does **not** consult `Lane::Trigger`, `Lane::Main`, or the per-agent semaphore at all.

Consequences:

1. With multiple adapters running concurrently (Telegram + Slack + Matrix + Discord …), in-flight dispatches reach `32 × N` and bypass the global `queue.concurrency.trigger_lane` cap (default 8) entirely — CLAUDE.md is explicit that "trigger_lane is the kernel-wide cap that prevents a runaway producer from spraying tokio tasks."
2. Worse, in the `select! { Some(message) => ... }` branch the **`tokio::spawn` happens before the permit is `acquire().await`'d** (`bridge.rs:1306-1338`) — only the dispatch concurrency is bounded; the **spawn rate itself is unbounded**. A webhook flood can produce hundreds of thousands of pending tasks in an instant, each holding an MiB-scale payload.

## Recommendation

Mirror the trigger-dispatcher pattern in `triggers_and_workflow.rs:416-428`:

1. Acquire the lane permit **before** `tokio::spawn`;
2. After routing to an agent, acquire the per-agent permit;
3. Treat the per-adapter 32 as a secondary fairness cap, not the primary rate limiter.

**Alternative** — a bounded mpsc with a fixed worker pool (from the earlier same-bug proposal):

```rust
let (tx, rx) = mpsc::channel::<InboundEvent>(trigger_lane * 4);
for _ in 0..N { spawn(worker(rx.clone())); }
```

Or `try_acquire` the lane semaphore on the dispatch side and shed-load (log + reject) when full, rather than queuing into a spawned task. The two approaches differ: the semaphore approach preserves the existing model and only relocates the queue head; the mpsc approach is a full producer-consumer rewrite that can cap peak memory. Pick one — do not stack them.

## Affected files (full sweep)

The inbound `select!` is not the only call site. The complete list (from the merged duplicate's investigation):
- `crates/librefang-channels/src/bridge.rs:1302, 1317, 1358, 1463, 3835, 8466`
- `crates/librefang-channels/src/telegram.rs:177, 1949, 1987`
- The same pattern in 12 other adapter files

`bridge.rs:3835` is particularly bad — the inbound dispatch path fans out one task per destination. The fix has to sweep every adapter.

## Tests

- Push 1000 messages through a single channel adapter; assert the in-flight task count never exceeds `trigger_lane + a small buffer`.
- High-volume burst (10k messages) → peak memory is bounded by `mpsc::channel` capacity rather than growing with sender rate.
