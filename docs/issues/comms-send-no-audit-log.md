# `comms_send` uses a byte-length limit, and cross-agent sends are not in the audit log

**Severity:** Low
**Category:** Input validation
**Labels:** `validation`, `audit`, `low`

## Affected files
- `crates/librefang-api/src/routes/network.rs:1996-2018`

## Description

Two small items:

1. `req.message.len() > 64 * 1024` repeats the byte-vs-char issue (see "message-byte-vs-char-cap");
2. `req.from_agent_id.parse()` validates the UUID, but **cross-agent sends are not audit-logged**. Every other privileged action (`routes/audit.rs:103-127`) lands in the hash-chained audit log.

Strictly speaking this is not an input-validation gap; it's a defense-in-depth gap.

## Recommendation

1. Switch to `chars().count()` to measure character count;
2. Call:

```rust
state.kernel.audit().record_with_context(
    "comms_send",
    json!({"from": from_agent_id, "to": to_agent_id, "len": msg.chars().count()}),
    /* ... */
).await?;
```

so cross-agent messages appear in the hash-chained audit log.
