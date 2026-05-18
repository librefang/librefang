# [High] `PooledDriver` does not invalidate the rate-limited key between retry attempts

**Severity:** High · **Domain:** LLM driver & MCP · **Source:** `audit-08-llm-mcp.md`

## Location
`crates/librefang-kernel/src/kernel/pooled_driver.rs:103-143`

## Problem
The driver-internal retry loop runs 3 attempts. The wrapper around it runs 2 attempts. With no key invalidation between them, a known-rate-limited key gets hammered **up to 6 times** before the wrapper marks it as exhausted and rotates to the next key.

This wastes the API budget, slows recovery, and inflates the user-visible latency on credentials-pool deployments (#5063).

## Fix
On the first `DriverError::RateLimit` from a key, immediately:
```rust
pool.mark_exhausted(&key_id);
let next = pool.acquire().ok_or(DriverError::AllKeysExhausted)?;
```
And restart the request on the new key rather than retrying the same one.

## Tests
- Unit: pool with 3 keys. Mock driver returns `RateLimit` on key A's first request. Assert pool moves to key B on attempt 2 (not retrying A 3 more times).
- Bench: rate-limit recovery latency drops from ~6× to ~2× the per-attempt RTT.
