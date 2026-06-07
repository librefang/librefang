# Change C-006 — Reload path reads DB store

**Phase:** phase-9-config-store-migration
**Status:** PLANNED
**Gap:** G-6 · **Effort:** M · **Depends on:** C-004 · **Agent:** claude

## Goal
`POST /api/config/reload` re-resolves effective config (bootstrap ⊕ DB) and
rebuilds the `ReloadPlan`; a DB-only `mcp_servers` change triggers
`HotAction::ReloadMcpServers` exactly as a file change did.

## Files
- `crates/librefang-kernel/src/kernel/config_reload_ops.rs`
- `crates/librefang-kernel/src/config_reload.rs` (`build_reload_plan`)

## ARCHITECTURE NOTE (revised per D9/D10, C-004)
Reload re-runs the API-layer overlay (shared pool + `SurrealConfigStore`) to
re-resolve `effective_mcp_servers` from the DB, then calls the kernel's existing
`reload_mcp_servers` / `connect_mcp_servers` to apply. The kernel does not read
the store directly.

## Tasks
- [ ] Reload re-resolves effective config from the store; DB change yields the
  same hot action (`ReloadMcpServers`).
- [ ] Integration test: change MCP server via DB store, reload, assert connection
  set updates.

## Done when
`cargo check -p librefang-kernel --lib` + `cargo test -p librefang-api` green;
reload reflects DB changes with no file edit.
