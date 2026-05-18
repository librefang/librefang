# [High] `dashboard_login` logs the freshly-computed Argon2id PHC string at INFO on legacy-plaintext upgrade

**Severity:** High · **Domain:** Auth & secrets · **Source:** `audit-01-auth-secrets.md`

## Location
`crates/librefang-api/src/server.rs:414-421`

```rust
tracing::info!(
    "Dashboard password verified via legacy plaintext. \
     Set `dashboard_pass_hash = \"{}\"` in config.toml \
     and remove `dashboard_pass` to complete the migration.",
    hash
);
```

## Problem
The `upgrade_hash` field is the Argon2id PHC string the daemon accepts as proof of password (`verify_dashboard_password` short-circuits on it at `password_hash.rs:214`). The hash **is** the verifier in this code path. Anyone with read access to the daemon log stream (journald, container stdout, log aggregator, Sentry) can:

1. Copy the PHC string from the log.
2. Write it into their own `config.toml: dashboard_pass_hash`.
3. Restart their daemon and authenticate as the victim operator.

No cracking required. Logs typically retain longer than passwords (no rotation story for log archives).

## Fix
Drop the hash from the log message. Surface the upgrade prompt through:
- a one-time stderr banner that displays the value to the operator's terminal only, or
- `/api/health/detail` (authenticated read), or
- a file at `~/.librefang/upgrade-hint.txt` with 0600 perms

## Tests
- Snapshot: log output on legacy-plaintext upgrade contains no `$argon2id$...` substring.
- Verify the upgrade hint is still discoverable through the chosen channel.
