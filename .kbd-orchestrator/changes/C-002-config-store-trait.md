# Change C-002 — `ConfigStore` trait + SurrealDB impl

**Phase:** phase-9-config-store-migration
**Status:** DONE (2026-06-07)
**Gap:** G-2 · **Effort:** M · **Depends on:** C-001 · **Agent:** claude

## What landed
- `crates/librefang-storage/src/config_store.rs` (new): `ConfigSource` enum,
  `ConfigEntry`, `ConfigStore` async trait (`get`/`list`/`upsert`/`delete`),
  `content_hash()` (object-key-order-independent canonical SHA-256), and
  `SurrealConfigStore` (one impl over `Surreal<Any>` — embedded + remote).
- `value` is enveloped as `{ "data": <v> }` so the `option<object>` column
  (migration v31) holds arrays/scalars/objects uniformly. Deterministic record
  id = SHA-256(key) for idempotent upsert/delete. `list()` is `ORDER BY key`.
- `crates/librefang-storage/src/lib.rs`: module + public re-exports.

## Verification (green)
- `cargo check -p librefang-storage --lib` → exit 0
- `cargo test -p librefang-storage --lib config_store` → 3 passed (embedded-DB
  round-trip upsert→get→list→delete with array + scalar values; hash order-
  independence; source parse)
- `cargo clippy -p librefang-storage --all-targets -- -D warnings` → clean
- `cargo check -p librefang-storage --lib --no-default-features --features sqlite-backend` → exit 0
- `enforce-branding.py --check` → clean

## Goal
A storage-layer abstraction for reading/writing system config, backed by the
existing `Surreal<Any>` handle (embedded + remote, one impl).

## Files
- `crates/librefang-storage/src/config_store.rs` (new)
- `crates/librefang-storage/src/lib.rs` (export)

## Tasks
- [ ] Trait: `get(key)`, `list(prefix)`, `upsert(key,value,source,content_hash,revision)`,
  `delete(key)`. `ConfigEntry { value, source, content_hash, revision, updated_at }`.
- [ ] Single `Surreal<Any>` impl; `list()` MUST `ORDER BY key` (determinism #3298).
- [ ] `#[cfg(feature = "sqlite-backend")]` parity is best-effort; surreal is the
  load-bearing default.

## Done when
`cargo check -p librefang-storage --lib` exit 0; round-trip unit test
(upsert→get→list→delete) on embedded DB passes.
