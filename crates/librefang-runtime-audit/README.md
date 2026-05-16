# librefang-runtime-audit

Tamper-evident audit log for [LibreFang](https://github.com/librefang/librefang) (refs #3710 Phase 1).

Every auditable runtime event is appended to a Merkle hash chain — each entry
holds the SHA-256 of its own contents concatenated with the prior entry's hash,
so any retroactive edit breaks the chain. When constructed `with_db`, entries
are persisted to the `audit_entries` table (SQLite, schema V8) and survive
daemon restarts.

## Where this fits

Extracted from `librefang-runtime` as part of the #3710 god-crate split.
`librefang-runtime` re-exports this crate at its historical path
(`runtime::audit`), so downstream call sites do not need to switch imports.

See the [workspace README](../../README.md) and `crates/librefang-runtime/README.md`.
