# Change C-004 — Kernel effective-config read path (bootstrap ⊕ DB)

**Phase:** phase-9-config-store-migration
**Status:** DONE (2026-06-07)
**Gap:** G-4 · **Effort:** M · **Depends on:** C-002 · **Agent:** claude

## ARCHITECTURE REVISION (user decision)
Discovery during execution: **the kernel holds no operational SurrealDB session
at boot.** Its `surreal_approval` / `surreal_totp` backends are constructed only
in tests; every operational-store access in the daemon is opened on-demand by the
**API/CLI layers**. Per the user's decision, the config store is **API-owned**:
the kernel exposes get/replace for `effective_mcp_servers`, and `librefang-api`
reads the store + overlays at boot. The original plan's "overlay inside the
kernel via `config.rs`" placement is superseded.

Also surfaced assessment **R-2** concretely: embedded RocksDB holds ONE lock per
path per process and does NOT release it on `SurrealConnectionPool` drop. Fixed
by adding a **process-global shared pool** (`librefang_storage::shared_pool()`)
that all daemon operational-store consumers reuse; migrated the two existing
on-demand `SurrealConnectionPool::new()` sites in `routes/storage.rs` to it so
the boot-time overlay does not deadlock embedded storage routes.

## What landed
- `crates/librefang-kernel/src/kernel/mcp_setup.rs`: public
  `effective_mcp_servers()` reader + `replace_effective_mcp_servers(Vec<…>)`
  (write lock + generation bump, mirrors `reload_mcp_servers` step 4).
- `crates/librefang-storage/src/pool.rs` + `lib.rs`: `shared_pool()` (OnceLock
  process-global `SurrealConnectionPool`).
- `crates/librefang-api/src/config_store_overlay.rs` (new): `overlay_mcp_servers`
  — opens via shared pool, applies pending migrations, reads `mcp_servers` from
  the store, pushes into the kernel. Best-effort (never blocks boot).
- `crates/librefang-api/src/server.rs`: wired into `run_daemon` between
  `set_self_handle()` and `start_background_agents()` (so the MCP-connect task
  sees the DB-resolved set), feature-gated on `surreal-backend`.
- `crates/librefang-api/src/routes/storage.rs`: two `new()` sites → `shared_pool()`.
- `crates/librefang-api/tests/config_store_overlay_test.rs` (new): two
  `#[tokio::test]` — DB entry overlays empty bootstrap; absent entry preserves
  bootstrap.

## Verification (green)
- `cargo check --workspace --lib` → exit 0
- `cargo test -p librefang-api --test config_store_overlay_test` → 2 passed
- `cargo test -p librefang-storage --lib` → 11 passed (incl. pool + config_store)
- `cargo clippy -p librefang-storage --all-targets -- -D warnings` → clean
- `cargo clippy -p librefang-kernel --lib` / `-p librefang-api --lib` → clean
  (modulo one PRE-EXISTING `large_enum_variant` lint in untouched
  `librefang-runtime`, surfaced only under feature-unification; not in changed code)
- `enforce-branding.py --check` → clean

## QA gate
8 files (≥3) → artifact-refiner gate applies. The `/refine-validate` skill is not
available in this session; QA is covered by the compile + clippy + integration-
test + brand gates above.
