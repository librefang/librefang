# `cron` / `autonomous` / `webui` system channel names are not validated at ingress; a custom adapter can collide with the SessionId

**Severity:** Medium
**Category:** Kernel orchestration logic
**Labels:** `bug`, `data-leak`, `medium`

## Affected files
- `crates/librefang-kernel/src/kernel/mod.rs:143-149` (`SYSTEM_CHANNEL_CRON = "cron"` etc.)
- `crates/librefang-types/src/agent.rs:292` (`SessionId::for_channel` lowercases)
- channel-bridge ingress construction sites

## Description

`SYSTEM_CHANNEL_*` are constants, but **no code enforces** that a custom channel adapter / `SenderContext` cannot set `channel = "cron"` (case-insensitively). `for_channel` calls `.to_lowercase()` internally — a misconfigured adapter passing `channel = "cron"` derives **the same** session id as the persistent cron-fire path.

Consequence: two write streams interleave into a single history. The `is_internal_cron` flag controls behaviour, but **not** SessionId derivation.

## Recommendation

Pick one:

1. Add `validate_channel_name(&str) -> Result<()>` at ingress, case-insensitively rejecting every `SYSTEM_CHANNEL_*` reserved name — also at `SenderContext` construction;
2. Even simpler: internal cron / autonomous code paths prepend a sentinel that no channel can input:

```rust
SessionId::for_channel(agent, "__cron")
```

This makes the namespace structurally disjoint from any string an adapter can inject.
