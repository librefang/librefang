# [High] `record_tool_calls` deque grows unbounded when limit disabled

**Severity:** High · **Domain:** Performance · **Source:** `audit-05-performance.md`

## Location
`crates/librefang-kernel/src/scheduler.rs:177-189`

## Problem
The scheduler records every tool call into a per-agent `VecDeque<Timestamp>` and evicts older entries on read. When `max_tool_calls_per_minute = 0` (the **default** in `KernelConfig`), the eviction read path is skipped — but the write path still pushes. The deque holds the full hour (or longer) of timestamps for any tool-heavy agent until daemon restart.

## Fix
Evict on push regardless of the limit, **or** short-circuit recording when the limit is disabled:

```rust
if self.max_tool_calls_per_minute == 0 {
    return; // no-op when limit disabled
}
deque.push_back(now);
while deque.front().map_or(false, |t| now.duration_since(*t) > Duration::from_secs(60)) {
    deque.pop_front();
}
```

## Tests
- 10k recorded calls with limit disabled → deque len ≤ small constant.
- 10k recorded calls with limit enabled → deque trimmed to ≤ N within the 60s window.
