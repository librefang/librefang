# Change C-002 — `ConfigStore` trait + SurrealDB impl

**Phase:** phase-9-config-store-migration
**Status:** PLANNED
**Gap:** G-2 · **Effort:** M · **Depends on:** C-001 · **Agent:** claude

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
