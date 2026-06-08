# Change C-005 — Re-target the MCP write path to `ConfigStore`

**Phase:** phase-9-config-store-migration
**Status:** DONE (2026-06-08)
**Gap:** G-5 (MCP slice) · **Effort:** M · **Depends on:** C-002, C-004 · **Agent:** claude

## Scope (user decision)
**MCP write path only** — defer provider-default (`persist_default_model`) and
generic `config_set` to a follow-up (C-005b). MCP writes go to the DB store
(`source="runtime"`), not config.toml; validation unchanged.

## What landed
- **kernel `mcp_setup.rs`**: evolved C-004's method into `replace_mcp_servers()`
  — now syncs **both** `config.mcp_servers` (via `ArcSwap::rcu`) **and** the
  effective list, so every existing reader (dup-checks, GET list, uninstall
  lookup) stays correct without changing those read sites.
- **kernel `kernel_api.rs`**: `replace_mcp_servers` added to the `KernelApi`
  trait + impl (handlers hold `Arc<dyn KernelApi>`).
- **api `config_store_overlay.rs`**: `write_mcp_servers(storage, servers, source)`
  helper (shared pool → apply migrations → upsert `mcp_servers` with content
  hash). Overlay updated to call `replace_mcp_servers`.
- **api `skills/mcp.rs`**: feature-gated `apply_mcp_mutation(state, McpMutation)`
  helper — surreal: store write + `replace_mcp_servers` + background reconnect;
  sqlite-only: legacy config.toml + `reload_config`. Converted all four handlers
  (add / update / patch-taint / delete); dropped config.toml writes +
  `reload_config` on the surreal path.
- **api `skills/extensions.rs`**: install/uninstall converted to the same helper
  (they register `mcp_servers`, so they are part of the MCP write path — leaving
  them on config.toml would still hit `os error 30` for extension installs in
  K8s).
- **api `skills/mod.rs`**: gated the now-legacy `upsert/remove_mcp_server_config`
  helpers + their 3 tests + the `json_to_toml_value` import on
  `#[cfg(not(feature = "surreal-backend"))]` (still used as the sqlite-only
  fallback).

## Files
- `crates/librefang-kernel/src/kernel/mcp_setup.rs`, `.../kernel_api.rs`
- `crates/librefang-api/src/config_store_overlay.rs`
- `crates/librefang-api/src/routes/skills/{mcp,extensions,mod}.rs`
- `crates/librefang-api/tests/config_store_overlay_test.rs` (C-005 test added)

## Verification
- `cargo check --workspace --lib` (default = surreal-backend)
- `cargo test -p librefang-api --test config_store_overlay_test` (incl. C-005
  `runtime_write_persists_and_survives_restart`)
- clippy + brand audit

## Deferred (tracked)
- **C-005b**: provider-default + generic `config_set` write paths (different
  subsystems; out of the user-chosen MCP-only scope).
- **Route-level HTTP TestServer** coverage of the MCP CRUD endpoints: the
  handlers are now thin wrappers over `write_mcp_servers` + `replace_mcp_servers`
  (both covered by the integration test against a real embedded SurrealDB). A
  full-router `oneshot` test needs a temp-storage variant of `start_full_router`
  (the existing harness uses CWD-relative storage) — folded into C-005b.

## Non-obvious decisions
- Sqlite-only api build is **pre-existing-broken** (`open_trace_store` cfg
  mismatch in `plugins.rs`/`plugin_manager`, untouched by this change), so the
  fallback path is gated correct-by-construction but not full-build-verified.
- `config_write_lock`: the MCP handlers never took it historically; the store
  upsert is a single keyed write, so no new lock added (matches prior behavior).
