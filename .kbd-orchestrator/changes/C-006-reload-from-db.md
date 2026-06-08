# Change C-006 — Reload path re-resolves from the DB store

**Phase:** phase-9-config-store-migration
**Status:** DONE (2026-06-08)
**Gap:** G-6 · **Effort:** M · **Depends on:** C-004 · **Agent:** claude

## Goal
`POST /api/config/reload` must not clobber DB-resolved MCP state. `reload_config`
re-reads config.toml and (via `ReloadMcpServers`) transiently resets the MCP
list to the bootstrap file values; C-006 re-runs the boot pipeline afterward so
the store stays authoritative and UI edits survive a reload.

## What landed
- `config/manage.rs` `config_reload`: after `reload_config()` succeeds, run
  **seed → overlay → reconcile** (surreal-gated): `seed_config_store` (picks up
  genuine config.toml changes, provenance-aware), `overlay_mcp_servers` (store →
  kernel), then `reload_mcp_servers` (reconcile live connections to the
  store-resolved list — connect new, disconnect removed).
- `config_store_overlay.rs`: widened `seed_config_store` and
  `overlay_mcp_servers` from `&LibreFangKernel` to `&dyn KernelApi` so the
  reload handler (which holds `Arc<dyn KernelApi>`) can call them; they only use
  trait methods (`config_ref`, `replace_mcp_servers`). Boot call sites coerce
  `&LibreFangKernel` automatically.

## Files
- `crates/librefang-api/src/routes/config/manage.rs`
- `crates/librefang-api/src/config_store_overlay.rs`
- `crates/librefang-api/tests/config_store_overlay_test.rs`

## Note
No kernel-side `build_reload_plan` change was needed (the original plan guessed
kernel files): with the API-owned design, reload re-resolution is purely the
API layer re-running the boot pipeline. The kernel's existing
`ReloadMcpServers`/`reload_mcp_servers` connection reconcile is reused.

## ARCHITECTURE NOTE (revised per D9/D10, C-004)
Reload re-runs the API-layer overlay (shared pool + `SurrealConfigStore`) to
re-resolve `effective_mcp_servers` from the DB, then calls the kernel's existing
`reload_mcp_servers` / `connect_mcp_servers` to apply. The kernel does not read
the store directly.

## Verification (green)
- `cargo test -p librefang-api --test config_store_overlay_test` → 8 passed
  (new: `reload_reresolve_preserves_runtime_over_bootstrap` — simulates
  `reload_config` resetting the kernel to bootstrap, then asserts the C-006
  re-resolve restores the runtime value and leaves the store row `runtime`).
- `cargo clippy -p librefang-api --lib` clean; brand audit clean.
