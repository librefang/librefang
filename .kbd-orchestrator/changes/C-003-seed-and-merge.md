# Change C-003 — Seed-once + content-hash/revision/provenance merge

**Phase:** phase-9-config-store-migration
**Status:** PLANNED
**Gap:** G-3 · **Effort:** M · **Depends on:** C-002 · **Agent:** claude

## Goal
Seed the DB from bootstrap config on first boot; re-sync ONLY on real content
change, per-key, provenance-aware. NO file mtime. NEVER clobber UI values.

## ARCHITECTURE NOTE (revised per D9, C-004)
Seed/merge runs in the **API layer**, not the kernel (the kernel has no
operational SurrealDB session). Use the shared pool + `SurrealConfigStore`. The
seed reads bootstrap values from `kernel.config_ref().*` and writes them into the
store with `source="bootstrap"`. Runs at boot in `run_daemon`, just BEFORE the
C-004 overlay (seed → overlay → connect), so the overlay reads a populated store.

## Files
- `crates/librefang-api/src/config_store_overlay.rs` (extend) OR a new
  `crates/librefang-api/src/config_store_seed.rs`
- `crates/librefang-api/src/server.rs` (call seed before overlay in `run_daemon`)

## Tasks
- [ ] Per in-scope section (§3a): hash bootstrap value; if no DB row → seed
  `source="bootstrap"` + hash + revision.
- [ ] If row exists: merge only when hash changed AND (row is `source="bootstrap"`
  OR bootstrap `revision` > stored `revision`). Never overwrite `source="runtime"`
  on a mere hash diff.
- [ ] No `std::fs` mtime read anywhere; code comment cites assessment FLAW 1.
- [ ] Unit tests: fresh seed / unchanged no-op / runtime-protected / revision-bump.

## Done when
`cargo check -p librefang-kernel --lib` exit 0; four merge tests pass under
`cargo test -p librefang-kernel config_store_sync`.
