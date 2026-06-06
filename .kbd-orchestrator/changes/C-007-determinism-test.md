# Change C-007 — Determinism `ORDER BY` + regression test

**Phase:** phase-9-config-store-migration
**Status:** PLANNED
**Gap:** G-7 · **Effort:** S · **Depends on:** C-004 · **Agent:** claude

## Goal
Guarantee prompt-reaching lists (`mcp_servers`) are byte-stable regardless of
DB row order — TOML insertion order is gone once it's rows (#3298).

## Files
- `crates/librefang-storage/src/config_store.rs` (confirm `ORDER BY key`)
- `crates/librefang-kernel/src/kernel/subsystems/mcp.rs`
- test beside existing `mcp_summary_*` tests

## Tasks
- [ ] Assert MCP summary byte-identical across DB row insertion orders (mirror
  `mcp_summary_is_byte_identical_across_input_orders`).

## Done when
New determinism test passes; `cargo test -p librefang-kernel` green.
