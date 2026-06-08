# Change C-001 — `config_store` SurrealDB migration

**Phase:** phase-9-config-store-migration
**Status:** DONE (2026-06-06)
**Gap:** G-1 · **Effort:** S · **Depends on:** none · **Agent:** claude

## Verification (green)
- `cargo check -p librefang-storage --lib` → exit 0
- `cargo test -p librefang-storage --test surreal_migration_invariants_test` → 3 passed
- `cargo test -p librefang-runtime --lib backends::surreal_audit::tests::append_and_verify_chain`
  → 1 passed (applies full migration set incl. v31 against embedded SurrealDB)
- `python3 scripts/enforce-branding.py --check` → exit 0

## Goal
Add a system-scoped `config_store` table (NOT the agent-scoped `kv_store`) to
hold runtime-mutable config with provenance + content-hash + revision fields.

## Files
- `crates/librefang-storage/src/migrations/sql/031_config_store.surql` (new)
- `crates/librefang-storage/src/migrations/mod.rs` (register `config_store_v1`)

## Tasks
- [ ] `DEFINE TABLE config_store SCHEMAFULL`; fields `key: string`,
  `value: option<object> FLEXIBLE`, `source: string`, `content_hash: string`,
  `revision: int`, `updated_at: string`; UNIQUE index on `key`.
- [ ] Register as `config_store_v1` after `030_composite_indexes`.
- [ ] Header comment: BossFang-exclusive table, no upstream SQLite equivalent.

## Done when
`cargo check -p librefang-storage --lib` exit 0; migration applies in a
`-p librefang-storage` embedded test.
