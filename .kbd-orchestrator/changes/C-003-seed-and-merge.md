# Change C-003 — Seed-once + content-hash/revision/provenance merge

**Phase:** phase-9-config-store-migration
**Status:** DONE (2026-06-08)
**Gap:** G-3 · **Effort:** M · **Depends on:** C-002 · **Agent:** claude

## Goal
Seed the DB from bootstrap config on first boot; re-sync ONLY on real content
change, per-key, provenance-aware. NO file mtime. NEVER clobber UI values.

## What landed
- `config_store_overlay.rs`: `seed_mcp_servers(storage, bootstrap, revision)`
  + `SeedOutcome` enum + `bootstrap_revision()` (reads
  `BOSSFANG_CONFIG_BOOTSTRAP_REVISION`, default 0) + boot wrapper
  `seed_config_store(kernel)`.
- Merge decision table (no mtime anywhere):
  - no row → **Seeded** (`source=bootstrap`)
  - same content hash → **Unchanged** (no write)
  - changed, stored=bootstrap → **BootstrapUpdated**
  - changed, stored=runtime, `revision > stored.revision` → **RevisionOverride**
  - changed, stored=runtime, otherwise → **RuntimeProtected** (UI edit kept)
- `server.rs`: wired `seed_config_store` BEFORE `overlay_mcp_servers` in
  `run_daemon` (seed → overlay → connect).
- The operator's only lever to override a UI edit is bumping
  `BOSSFANG_CONFIG_BOOTSTRAP_REVISION` (a ConfigMap env in K8s) — a content
  change alone never clobbers runtime rows (assessment FLAW 1 + FLAW 2).

## Verification (green)
- `cargo test -p librefang-api --test config_store_overlay_test` → 7 passed
  (4 new: fresh-seed/unchanged, bootstrap-update, runtime-protected,
  revision-override).
- `cargo clippy -p librefang-api --lib` clean; brand audit clean.

## Note on scope
Seeds the `mcp_servers` key (the only in-scope key after the MCP-only C-005).
Provider-default + config_set seeding folds into **C-005b** alongside their
write paths.

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
