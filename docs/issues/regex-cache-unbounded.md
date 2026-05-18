# Router regex-compilation cache grows unbounded

**Severity:** Medium
**Category:** DoS / resource exhaustion
**Labels:** `dos`, `memory-leak`, `medium`

## Affected files
- `crates/librefang-kernel-router/src/lib.rs:980-994`

## Description

```rust
REGEX_CACHE: OnceLock<Mutex<HashMap<String, Regex>>>
```

The key is the raw pattern string, user/config-controlled. Every new pattern enters the cache and is **never evicted**. Each entry holds the source string + the compiled `Regex` (multiple KB).

Source chain: agent route / manifest patterns pass through `regex_matches(message, pattern)`. An operator or a malicious agent can keep pumping new patterns and inflate the cache without bound.

## Recommendation

Pick one:

1. Switch to LRU:

```rust
use lru::LruCache;
static REGEX_CACHE: OnceLock<Mutex<LruCache<String, Regex>>> = OnceLock::new();
// capacity 4096
```

2. Stop memoizing on match: validate + precompile at config load and store inside the router instance as an immutable `Vec`.

The latter is more thorough but requires auditing all call sites.
