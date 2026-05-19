# Agent identity bootstrap silently swallows file-write failures

**Severity:** Medium
**Category:** Panic / error handling
**Labels:** `bug`, `data-integrity`, `medium`

## Affected files
- `crates/librefang-kernel/src/kernel/workspace_setup.rs:517, 533, 548`

## Description

```rust
let _ = f.write_all(content.as_bytes());
```

is called after a successful `OpenOptions::create_new(...).open(...)`. When the write fails mid-stream (ENOSPC, EIO, EDQUOT):

- The file is left empty or truncated;
- The caller receives `Ok`;
- After agent spawn, SOUL.md / TOOLS.md / HEARTBEAT.md are empty;
- On the next spawn, `create_new` refuses to overwrite the corrupted file — permanently stuck.

## Recommendation

Pick one:

1. `f.write_all(...)?` to propagate the error directly;
2. At minimum, `tracing::warn!` and delete the partial file so the next spawn retries.

The TOOLS.md branch at `:536` already handles `OpenOptions::open` failure correctly — apply the same pattern to the write step.
