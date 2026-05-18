# [High] Trigger concurrency caps have resolver tests but NO end-to-end parallel-firing test

**Severity:** High · **Domain:** Test coverage · **Source:** `audit-07-test-coverage.md`

## Location
`crates/librefang-kernel/src/kernel/tests.rs:4557-4710` — resolver math is tested. No test actually fires N parallel dispatches and verifies the semaphore enforces the cap.

## Problem
CLAUDE.md describes three layered caps (`Lane::Trigger` global, per-agent `max_concurrent_invocations`, per-session mutex) plus auto-clamp logic for `persistent + cap > 1 → 1`. The resolver tests check that the right cap is computed from the right inputs. None of them test that, when you actually fire 10 trigger dispatches at once, the semaphore actually holds 9 of them.

This means the concurrency-hygiene cluster (cost ledger race in "cost-reservation-not-atomic", manifest-snapshot drift in "trigger-dispatch-two-snapshots", and BUSY_SNAPSHOT under contention) all could regress without any test catching them.

## Fix
Add `#[tokio::test(flavor = "multi_thread", worker_threads = 8)]`:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn trigger_lane_global_cap_enforced() {
    let kernel = MockKernelBuilder::new()
        .with_concurrency(Lane::Trigger, 2)
        .build().await;
    let agent = kernel.spawn_test_agent_with_slow_tool(Duration::from_millis(500));
    let started = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    // fire 10 in parallel, sample peak in-flight count
    // assert peak <= 2
}
```

Cover:
- Global trigger lane cap.
- Per-agent cap.
- Per-session mutex (when `session_mode = "new"`).
- Auto-clamp on `persistent + cap > 1`.

## Tests
The test above; mark as `#[ignore]` only if it adds > 5s to the suite.
