# [Medium] `DriverCache::cache_key` uses 64-bit `DefaultHasher` digest of api_key — birthday collisions possible

**Severity:** Medium · **Domain:** LLM driver & MCP · **Source:** `audit-08-llm-mcp.md`

## Location
`crates/librefang-llm-drivers/src/drivers/mod.rs:86-105`

## Problem
A 64-bit digest is subject to birthday collisions at ~2³² entries. With credential pools (#5063) supporting hundreds of keys per provider across multiple instances, deliberate-collision risk is non-zero. Worst case: cache returns a driver instance built for a different API key.

## Fix
Use SHA-256 (or just key the cache by the full key string):
```rust
let key = sha256::digest(api_key); // first 16 bytes as cache key
```
Or `HashMap<String, Arc<Driver>>` keyed by full key (cache size is bounded by pool size anyway).

## Tests
- Collision test: insert N entries with constructed colliding 64-bit hashes (find via brute force in test fixture) → assert no cache cross-contamination after the fix.
