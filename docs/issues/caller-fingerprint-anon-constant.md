# [Medium] `caller_fingerprint(&None)` returns a CONSTANT `SHA256("anon")[..16]`

**Severity:** Medium · **Domain:** Auth & secrets · **Source:** `audit-01-auth-secrets.md`

## Location
`crates/librefang-api/src/routes/mcp_auth.rs:28-40`

## Problem
When no `[[users]]` are configured, every flow falls into the `anon` branch. All anonymous MCP OAuth flows then share **one vault namespace**. The random `flow_id` distinguishes concurrent flows within one daemon (so not directly exploitable in the documented threat model), but an attacker with loopback API access (malicious npm/cargo dev tool in the same shell) can iterate `auth_start` against MCP servers, read `state` from the returned `auth_url`, and race the callback. The per-user binding designed at `:22-27` has no effect.

## Fix
When no authenticated identity is attached, derive fingerprint from a per-process random nonce (set at boot via `OsRng`) instead of a constant:
```rust
static ANON_FINGERPRINT: LazyLock<[u8; 16]> = LazyLock::new(|| {
    let mut buf = [0u8; 16];
    OsRng.fill_bytes(&mut buf);
    buf
});
```

## Tests
- Two daemon restarts produce different anonymous fingerprints (verify via observable side effect, e.g. vault namespace).
