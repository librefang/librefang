# Change C-007 — Determinism `ORDER BY` + regression test

**Phase:** phase-9-config-store-migration
**Status:** DONE (2026-06-08)
**Gap:** G-7 · **Effort:** S · **Depends on:** C-004 · **Agent:** claude

## Goal
Guarantee prompt-reaching config lists are byte-stable regardless of storage
iteration order (#3298).

## Structural finding (premise correction)
The original plan assumed **row-per-server** storage where TOML insertion order
is lost and `ORDER BY` is load-bearing for the prompt. Our design (C-002) stores
`mcp_servers` as a **single row holding an ordered JSON array**, so:
- The array order is exactly the write order — no DB-row-order nondeterminism
  for `mcp_servers`.
- The prompt-reaching path `build_mcp_summary → render_mcp_summary` **already
  sorts** servers + tools lexicographically, so the rendered summary is
  byte-identical regardless of the stored array order. This is covered by the
  existing kernel tests `mcp_summary_is_byte_identical_across_input_orders` /
  `mcp_summary_inner_tool_list_is_sorted` (verified, unchanged).

So no kernel change was needed. The remaining genuine guard is the config
store's `list()` ordering — the future multi-key, prompt-reaching read path.

## What landed
- `crates/librefang-storage/src/config_store.rs`: regression test
  `list_is_sorted_by_key_regardless_of_insertion_order` — inserts keys
  `z_last`/`a_first`/`m_middle` out of order and asserts `list()` returns them
  sorted (drop the `ORDER BY key` and RocksDB returns insertion order → the test
  fails). Also asserts prefix queries are sorted.

## Verification (green)
- `cargo test -p librefang-storage --lib config_store` → 4 passed (incl. new
  determinism guard).
- `cargo clippy -p librefang-storage --all-targets -- -D warnings` clean; brand
  audit clean.
- Existing kernel `mcp_summary_*` tests already cover prompt byte-stability
  (unchanged by this phase).
