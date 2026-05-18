# [High] `audit_export` streaming swallows `serde_json::to_writer` errors → malformed JSON possible

**Severity:** High · **Domain:** Error handling · **Source:** `audit-04-error-handling.md`

## Location
`crates/librefang-api/src/routes/audit.rs:380-388`

## Problem
The shape is:
```rust
let mut buf = Vec::new();
buf.push(b',');                                  // separator first
let _ = serde_json::to_writer(&mut buf, &value); // ignored
```
If `to_writer` fails, the chunk contains only `,` — producing output like `[ , {...}, , {...} ]`, which is **invalid JSON**. Even if `to_writer` into a `Vec<u8>` is practically infallible (no I/O), the `let _ =` masks the invariant; any future refactor that swaps the writer for a real `tokio::io::AsyncWrite` instantly silently corrupts exports.

## Fix
Either:
```rust
serde_json::to_writer(&mut buf, &value)
    .expect("serializing serde_json::Value into Vec<u8> is infallible");
```
or build via `to_vec` and concatenate:
```rust
let payload = serde_json::to_vec(&value).expect("infallible");
if !first { buf.push(b','); }
buf.extend_from_slice(&payload);
```

## Tests
- Property test: round-trip arbitrary audit rows through export → `serde_json::from_slice::<Vec<Value>>` succeeds for every output.
