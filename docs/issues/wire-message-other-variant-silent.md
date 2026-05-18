# `WireMessage*` `#[serde(other)]` arm silently swallows unknown variants — no log, no counter

**Severity:** Medium
**Category:** Input validation
**Labels:** `validation`, `observability`, `peer-misbehavior`, `medium`

## Affected files
- `crates/librefang-wire/src/message.rs:34, 103, 158, 176` (four `#[serde(other)]` sites)

## Description

The comment claims "forward-compat fallback," which is fine in principle. The problem:

- The receive loop **silently** drops every unknown or malformed message;
- A peer can flood `{"type":"garbage","method":"x"}` and every message deserializes as `Unknown`;
- No log, no counter, no peer-misbehaviour metric;
- Combined with `#[serde(default)] nonce: String` / `auth_hmac: String` at `message.rs:54-58`, a peer that sends an unknown handshake variant can bypass structural checks.

## Recommendation

Replace `Unknown` with a type-tagged variant:

```rust
Unrecognized { raw_type: String }
```

At the receive site:

```rust
warn!(target = "wire::compat", peer = %peer_id, msg_type = %raw_type,
      "unknown wire message");
```

Add a per-peer counter; disconnect when the threshold is exceeded.
