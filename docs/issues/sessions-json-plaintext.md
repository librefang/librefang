# [Medium] `sessions.json` stores random session tokens plaintext at rest

**Severity:** Medium · **Domain:** Auth & secrets · **Source:** `audit-01-auth-secrets.md`

## Location
- `crates/librefang-api/src/server.rs:944-987` (`save_sessions`)
- TTL: `password_hash.rs:118` (`DEFAULT_SESSION_TTL_SECS = 30 * 24 * 3600`)

## Problem
Session tokens are 32-byte CSPRNG outputs protected by `0600` perms (#3725 hardened this). But the 30-day TTL means **any backup snapshot from the last month is a valid set of bearer tokens**. Backup pipelines (Time Machine, restic, BorgBackup) may not honor source perms. No per-token revocation API beyond global logout.

The `password_hash` module already has the primitive (`hash_device_token`).

## Fix
Hash the session token before persisting:
```rust
let stored = hash_device_token(&token);
sessions.insert(stored, metadata);
```
In-memory `active_sessions` keeps the cleartext token; disk-recovered tokens can no longer be replayed.

## Tests
- `cat sessions.json` shows no recognizable session-token format.
- Restore from backup → all tokens invalidated unless recreated.
