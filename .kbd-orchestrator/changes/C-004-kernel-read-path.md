# Change C-004 — Kernel effective-config read path (bootstrap ⊕ DB)

**Phase:** phase-9-config-store-migration
**Status:** PLANNED
**Gap:** G-4 · **Effort:** M · **Depends on:** C-002 · **Agent:** claude

## Goal
Resolve effective config = bootstrap defaults overlaid with DB overrides, at
boot. Populate `effective_mcp_servers` from the DB, not `cfg.mcp_servers`.

## Files
- `crates/librefang-kernel/src/config.rs`
- `crates/librefang-kernel/src/kernel/subsystems/mcp.rs`

## Tasks
- [ ] Overlay DB `config_store` in-scope keys onto bootstrap `KernelConfig` →
  effective config in the existing `ArcSwap`.
- [ ] Fill `effective_mcp_servers` RwLock from the overlaid value.
- [ ] Out-of-scope sections (§3b) read straight from file/env, untouched.

## Done when
`cargo check -p librefang-kernel --lib` exit 0; test asserts a DB-stored MCP
server appears in `effective_mcp_servers` with empty `cfg.mcp_servers`.
